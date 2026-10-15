use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};

use crate::media_cache::{cache_note_media, remove_note_media};

pub struct LibraryState {
    pub conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub id: i64,
    pub platform: String,
    pub item_id: String,
    pub title: String,
    pub author_name: String,
    pub likes: i64,
    pub comments: i64,
    pub collects: i64,
    pub shares: i64,
    pub url: String,
    pub cover_url: String,
    pub cover_local_path: String,
    pub images_json: String,
    pub images_local_json: String,
    pub desc: String,
    pub published_at: String,
    pub raw_json: String,
    pub saved_at: String,
    pub group_id: String,
    pub analysis_json: String,
    pub analyzed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryGroup {
    pub id: String,
    pub platform: String,
    pub name: String,
    pub sort_order: i64,
    pub analysis_json: String,
    pub analyzed_at: String,
    pub created_at: String,
    pub updated_at: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryGroupListResult {
    pub groups: Vec<LibraryGroup>,
    pub total_count: i64,
    pub ungrouped_count: i64,
    pub ungrouped_analysis_json: String,
    pub ungrouped_analyzed_at: String,
}

fn db_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir app_data: {e}"))?;
    Ok(dir.join("library.sqlite3"))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn new_group_id() -> String {
    format!("lg_{}", now_ms())
}

fn ungrouped_sys_id(platform: &str) -> String {
    format!("{platform}:sys:ungrouped")
}

pub fn init_library(app: &AppHandle) -> Result<LibraryState, String> {
    let path = db_path(app)?;
    let conn = Connection::open(&path).map_err(|e| format!("open db: {e}"))?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode=WAL;
        CREATE TABLE IF NOT EXISTS library_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            platform TEXT NOT NULL,
            item_id TEXT NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            author_name TEXT NOT NULL DEFAULT '',
            likes INTEGER NOT NULL DEFAULT 0,
            comments INTEGER NOT NULL DEFAULT 0,
            collects INTEGER NOT NULL DEFAULT 0,
            shares INTEGER NOT NULL DEFAULT 0,
            url TEXT NOT NULL DEFAULT '',
            cover_url TEXT NOT NULL DEFAULT '',
            desc_text TEXT NOT NULL DEFAULT '',
            published_at TEXT NOT NULL DEFAULT '',
            raw_json TEXT NOT NULL DEFAULT '{}',
            saved_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(platform, item_id)
        );
        CREATE INDEX IF NOT EXISTS idx_library_platform ON library_items(platform);

        CREATE TABLE IF NOT EXISTS llm_provider (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            provider TEXT NOT NULL,
            modality TEXT NOT NULL DEFAULT 'text',
            base_url TEXT NOT NULL,
            api_key TEXT NOT NULL DEFAULT '',
            api_key_hint TEXT NOT NULL DEFAULT '',
            default_model TEXT NOT NULL DEFAULT '',
            extra_models TEXT NOT NULL DEFAULT '[]',
            timeout_sec INTEGER NOT NULL DEFAULT 120,
            enabled INTEGER NOT NULL DEFAULT 1,
            is_default INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS drafts (
            id TEXT PRIMARY KEY NOT NULL,
            platform TEXT NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            body TEXT NOT NULL DEFAULT '',
            tags_json TEXT NOT NULL DEFAULT '[]',
            image_urls_json TEXT NOT NULL DEFAULT '[]',
            source_item_id TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'editing',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_drafts_platform ON drafts(platform);

        CREATE TABLE IF NOT EXISTS publish_logs (
            id TEXT PRIMARY KEY NOT NULL,
            platform TEXT NOT NULL,
            draft_id TEXT NOT NULL DEFAULT '',
            title TEXT NOT NULL DEFAULT '',
            action TEXT NOT NULL DEFAULT '',
            ok INTEGER NOT NULL DEFAULT 0,
            message TEXT NOT NULL DEFAULT '',
            steps_json TEXT NOT NULL DEFAULT '[]',
            profile_id TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_publish_logs_platform_created
            ON publish_logs(platform, created_at DESC);

        "#,
    )
    .map_err(|e| format!("migrate: {e}"))?;
    ensure_library_media_columns(&conn)?;
    ensure_library_group_schema(&conn)?;
    crate::competitors::ensure_competitor_schema(&conn)?;
    crate::browser_cdp::ensure_browser_table(&conn)?;
    crate::knowledge::ensure_auto_ops_table(&conn)?;
    crate::knowledge::ensure_cs_chat_tables(&conn)?;
    crate::assets::ensure_assets_table(&conn)?;
    crate::image_jobs::ensure_image_jobs_table(&conn)?;
    let _ = crate::image_jobs::mark_interrupted_jobs(&conn);
    let _ = crate::assets::assets_root(app);
    Ok(LibraryState {
        conn: Mutex::new(conn),
    })
}

fn table_has_column(conn: &Connection, table: &str, col: &str) -> bool {
    let mut stmt = match conn.prepare(&format!("PRAGMA table_info({table})")) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let rows = stmt.query_map([], |r| r.get::<_, String>(1));
    match rows {
        Ok(iter) => iter.flatten().any(|name| name == col),
        Err(_) => false,
    }
}

pub(crate) fn ensure_library_media_columns(conn: &Connection) -> Result<(), String> {
    let alters = [
        ("cover_local_path", "TEXT NOT NULL DEFAULT ''"),
        ("images_json", "TEXT NOT NULL DEFAULT '[]'"),
        ("images_local_json", "TEXT NOT NULL DEFAULT '[]'"),
    ];
    for (col, typ) in alters {
        if !table_has_column(conn, "library_items", col) {
            conn.execute(
                &format!("ALTER TABLE library_items ADD COLUMN {col} {typ}"),
                [],
            )
            .map_err(|e| format!("alter library_items.{col}: {e}"))?;
        }
    }
    Ok(())
}

pub(crate) fn ensure_library_group_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS library_groups (
            id TEXT PRIMARY KEY NOT NULL,
            platform TEXT NOT NULL,
            name TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            analysis_json TEXT NOT NULL DEFAULT '',
            analyzed_at TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(platform, name)
        );
        CREATE INDEX IF NOT EXISTS idx_library_groups_platform
            ON library_groups(platform, sort_order ASC, name COLLATE NOCASE);
        "#,
    )
    .map_err(|e| format!("library_groups: {e}"))?;

    let alters = [
        ("group_id", "TEXT NOT NULL DEFAULT ''"),
        ("analysis_json", "TEXT NOT NULL DEFAULT ''"),
        ("analyzed_at", "TEXT NOT NULL DEFAULT ''"),
    ];
    for (col, typ) in alters {
        if !table_has_column(conn, "library_items", col) {
            conn.execute(
                &format!("ALTER TABLE library_items ADD COLUMN {col} {typ}"),
                [],
            )
            .map_err(|e| format!("alter library_items.{col}: {e}"))?;
        }
    }
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_library_platform_group
            ON library_items(platform, group_id);
        "#,
    )
    .map_err(|e| format!("idx group: {e}"))?;
    Ok(())
}

fn parse_image_urls(item: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<String>, v: &Value| {
        if let Some(s) = v.as_str() {
            let t = s.trim();
            if !t.is_empty() && !out.iter().any(|x| x == t) {
                out.push(t.to_string());
            }
        } else if let Some(obj) = v.as_object() {
            for k in ["url", "urlDefault", "url_default"] {
                if let Some(s) = obj.get(k).and_then(|x| x.as_str()) {
                    let t = s.trim();
                    if !t.is_empty() && !out.iter().any(|x| x == t) {
                        out.push(t.to_string());
                    }
                    break;
                }
            }
            if let Some(arr) = obj.get("url_list").and_then(|x| x.as_array()) {
                if let Some(s) = arr.first().and_then(|x| x.as_str()) {
                    let t = s.trim();
                    if !t.is_empty() && !out.iter().any(|x| x == t) {
                        out.push(t.to_string());
                    }
                }
            }
        }
    };
    for key in ["imageUrls", "images", "image_list", "imageList"] {
        if let Some(arr) = item.get(key).and_then(|x| x.as_array()) {
            for v in arr {
                push(&mut out, v);
            }
        }
    }
    out
}

fn json_str(v: &Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(|x| x.as_str()) {
            return s.to_string();
        }
    }
    String::new()
}

fn json_i64(v: &Value, keys: &[&str]) -> i64 {
    for k in keys {
        if let Some(n) = v.get(*k).and_then(|x| x.as_i64()) {
            return n;
        }
        if let Some(n) = v.get(*k).and_then(|x| x.as_f64()) {
            return n as i64;
        }
        if let Some(s) = v.get(*k).and_then(|x| x.as_str()) {
            if let Ok(n) = s.parse::<i64>() {
                return n;
            }
        }
    }
    0
}

fn map_library_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<LibraryItem> {
    Ok(LibraryItem {
        id: row.get(0)?,
        platform: row.get(1)?,
        item_id: row.get(2)?,
        title: row.get(3)?,
        author_name: row.get(4)?,
        likes: row.get(5)?,
        comments: row.get(6)?,
        collects: row.get(7)?,
        shares: row.get(8)?,
        url: row.get(9)?,
        cover_url: row.get(10)?,
        cover_local_path: row.get(11)?,
        images_json: row.get(12)?,
        images_local_json: row.get(13)?,
        desc: row.get(14)?,
        published_at: row.get(15)?,
        raw_json: row.get(16)?,
        saved_at: row.get(17)?,
        group_id: row.get::<_, Option<String>>(18)?.unwrap_or_default(),
        analysis_json: row.get::<_, Option<String>>(19)?.unwrap_or_default(),
        analyzed_at: row.get::<_, Option<String>>(20)?.unwrap_or_default(),
    })
}

const ITEM_SELECT: &str = r#"
    SELECT id, platform, item_id, title, author_name, likes, comments, collects, shares,
           url, cover_url,
           COALESCE(cover_local_path,''), COALESCE(images_json,'[]'), COALESCE(images_local_json,'[]'),
           desc_text, published_at, raw_json, saved_at,
           COALESCE(group_id,''), COALESCE(analysis_json,''), COALESCE(analyzed_at,'')
    FROM library_items
"#;

#[tauri::command]
pub fn library_save_items(
    app: AppHandle,
    state: State<'_, LibraryState>,
    platform: String,
    items: Vec<Value>,
) -> Result<Value, String> {
    let plat = platform.trim().to_lowercase();
    if plat != "xhs" && plat != "douyin" {
        return Err(format!("unsupported platform: {platform}"));
    }
    let mut saved = 0i64;
    for item in items {
        let item_id = json_str(&item, &["id", "itemId", "noteId"]);
        if item_id.is_empty() {
            continue;
        }
        let title = json_str(&item, &["title"]);
        let author_name = json_str(&item, &["authorName", "author_name"]);
        let likes = json_i64(&item, &["likes"]);
        let comments = json_i64(&item, &["comments"]);
        let collects = json_i64(&item, &["collects"]);
        let shares = json_i64(&item, &["shares"]);
        let url = json_str(&item, &["url", "noteUrl"]);
        let cover_url = json_str(&item, &["coverUrl", "cover_url"]);
        let desc = json_str(&item, &["desc", "content", "description"]);
        let published_at = json_str(&item, &["publishedAt", "createdAt", "created_at"]);
        let image_urls = parse_image_urls(&item);
        let images_json = serde_json::to_string(&image_urls).unwrap_or_else(|_| "[]".into());
        let raw_json = serde_json::to_string(&item).unwrap_or_else(|_| "{}".into());

        let cached = cache_note_media(
            &app,
            &format!("library-{plat}"),
            &item_id,
            &cover_url,
            &image_urls,
        );
        let cover_local = cached.cover_local;
        let images_local_json =
            serde_json::to_string(&cached.images_local).unwrap_or_else(|_| "[]".into());

        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        ensure_library_media_columns(&conn)?;
        ensure_library_group_schema(&conn)?;
        // 冲突更新不覆盖 group_id / analysis
        conn.execute(
            r#"
            INSERT INTO library_items (
                platform, item_id, title, author_name, likes, comments, collects, shares,
                url, cover_url, cover_local_path, images_json, images_local_json,
                desc_text, published_at, raw_json, saved_at
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16, datetime('now'))
            ON CONFLICT(platform, item_id) DO UPDATE SET
                title=excluded.title,
                author_name=excluded.author_name,
                likes=excluded.likes,
                comments=excluded.comments,
                collects=excluded.collects,
                shares=excluded.shares,
                url=excluded.url,
                cover_url=excluded.cover_url,
                cover_local_path=excluded.cover_local_path,
                images_json=excluded.images_json,
                images_local_json=excluded.images_local_json,
                desc_text=excluded.desc_text,
                published_at=excluded.published_at,
                raw_json=excluded.raw_json,
                saved_at=datetime('now')
            "#,
            params![
                plat,
                item_id,
                title,
                author_name,
                likes,
                comments,
                collects,
                shares,
                url,
                cover_url,
                cover_local,
                images_json,
                images_local_json,
                desc,
                published_at,
                raw_json
            ],
        )
        .map_err(|e| format!("upsert: {e}"))?;
        drop(conn);
        saved += 1;
    }
    Ok(serde_json::json!({ "ok": true, "saved": saved }))
}

/// group_id: None=全部；Some("")=未分组；Some(id)=指定组
#[tauri::command]
pub fn library_list(
    state: State<'_, LibraryState>,
    platform: String,
    group_id: Option<String>,
) -> Result<Vec<LibraryItem>, String> {
    let plat = platform.trim().to_lowercase();
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    ensure_library_media_columns(&conn)?;
    ensure_library_group_schema(&conn)?;

    let (sql, bind_group): (String, Option<String>) = match group_id {
        None => (
            format!("{ITEM_SELECT} WHERE platform = ?1 ORDER BY saved_at DESC, id DESC"),
            None,
        ),
        Some(ref g) if g.trim().is_empty() => (
            format!(
                "{ITEM_SELECT} WHERE platform = ?1 AND COALESCE(group_id,'') = '' \
                 ORDER BY saved_at DESC, id DESC"
            ),
            None,
        ),
        Some(g) => (
            format!(
                "{ITEM_SELECT} WHERE platform = ?1 AND COALESCE(group_id,'') = ?2 \
                 ORDER BY saved_at DESC, id DESC"
            ),
            Some(g.trim().to_string()),
        ),
    };

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mapped = if let Some(gid) = bind_group {
        stmt.query_map(params![plat, gid], map_library_item)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
    } else {
        stmt.query_map(params![plat], map_library_item)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
    };
    mapped.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn library_delete(
    app: AppHandle,
    state: State<'_, LibraryState>,
    platform: String,
    item_id: String,
) -> Result<Value, String> {
    let plat = platform.trim().to_lowercase();
    let item_id = item_id.trim().to_string();
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let n = conn
        .execute(
            "DELETE FROM library_items WHERE platform = ?1 AND item_id = ?2",
            params![plat, item_id],
        )
        .map_err(|e| e.to_string())?;
    drop(conn);
    remove_note_media(&app, &format!("library-{plat}"), &item_id);
    Ok(serde_json::json!({ "ok": true, "deleted": n }))
}

#[tauri::command]
pub fn library_group_list(
    state: State<'_, LibraryState>,
    platform: String,
) -> Result<LibraryGroupListResult, String> {
    let plat = platform.trim().to_lowercase();
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    ensure_library_group_schema(&conn)?;

    let total_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM library_items WHERE platform = ?1",
            params![plat],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let ungrouped_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM library_items WHERE platform = ?1 AND COALESCE(group_id,'') = ''",
            params![plat],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let sys_id = ungrouped_sys_id(&plat);
    let (ungrouped_analysis_json, ungrouped_analyzed_at): (String, String) = conn
        .query_row(
            "SELECT COALESCE(analysis_json,''), COALESCE(analyzed_at,'') FROM library_groups WHERE id = ?1",
            params![sys_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or_else(|_| (String::new(), String::new()));

    let mut stmt = conn
        .prepare(
            r#"
            SELECT g.id, g.platform, g.name, g.sort_order,
                   COALESCE(g.analysis_json,''), COALESCE(g.analyzed_at,''),
                   g.created_at, g.updated_at,
                   (SELECT COUNT(*) FROM library_items i
                    WHERE i.platform = g.platform AND COALESCE(i.group_id,'') = g.id)
            FROM library_groups g
            WHERE g.platform = ?1 AND g.id != ?2
            ORDER BY g.sort_order ASC, g.name COLLATE NOCASE ASC
            "#,
        )
        .map_err(|e| e.to_string())?;
    let groups = stmt
        .query_map(params![plat, sys_id], |row| {
            Ok(LibraryGroup {
                id: row.get(0)?,
                platform: row.get(1)?,
                name: row.get(2)?,
                sort_order: row.get(3)?,
                analysis_json: row.get(4)?,
                analyzed_at: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                count: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(LibraryGroupListResult {
        groups,
        total_count,
        ungrouped_count,
        ungrouped_analysis_json,
        ungrouped_analyzed_at,
    })
}

#[tauri::command]
pub fn library_group_create(
    state: State<'_, LibraryState>,
    platform: String,
    name: String,
) -> Result<LibraryGroup, String> {
    let plat = platform.trim().to_lowercase();
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("分组名称不能为空".into());
    }
    if name == "未分组" || name == "全部" {
        return Err("该名称不可用".into());
    }
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    ensure_library_group_schema(&conn)?;
    let next_order: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM library_groups WHERE platform = ?1",
            params![plat],
            |r| r.get(0),
        )
        .unwrap_or(1);
    let id = new_group_id();
    conn.execute(
        r#"
        INSERT INTO library_groups (id, platform, name, sort_order, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))
        "#,
        params![id, plat, name, next_order],
    )
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            "分组名称已存在".into()
        } else {
            format!("创建分组失败: {e}")
        }
    })?;
    Ok(LibraryGroup {
        id,
        platform: plat,
        name,
        sort_order: next_order,
        analysis_json: String::new(),
        analyzed_at: String::new(),
        created_at: String::new(),
        updated_at: String::new(),
        count: 0,
    })
}

#[tauri::command]
pub fn library_group_rename(
    state: State<'_, LibraryState>,
    platform: String,
    id: String,
    name: String,
) -> Result<(), String> {
    let plat = platform.trim().to_lowercase();
    let id = id.trim().to_string();
    let name = name.trim().to_string();
    if id.is_empty() || id == ungrouped_sys_id(&plat) {
        return Err("无法重命名该分组".into());
    }
    if name.is_empty() {
        return Err("分组名称不能为空".into());
    }
    if name == "未分组" || name == "全部" {
        return Err("该名称不可用".into());
    }
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    ensure_library_group_schema(&conn)?;
    let n = conn
        .execute(
            r#"
            UPDATE library_groups SET name = ?3, updated_at = datetime('now')
            WHERE id = ?1 AND platform = ?2
            "#,
            params![id, plat, name],
        )
        .map_err(|e| {
            if e.to_string().contains("UNIQUE") {
                "分组名称已存在".into()
            } else {
                format!("重命名失败: {e}")
            }
        })?;
    if n == 0 {
        return Err("分组不存在".into());
    }
    Ok(())
}

#[tauri::command]
pub fn library_group_delete(
    state: State<'_, LibraryState>,
    platform: String,
    id: String,
) -> Result<Value, String> {
    let plat = platform.trim().to_lowercase();
    let id = id.trim().to_string();
    if id.is_empty() || id == ungrouped_sys_id(&plat) {
        return Err("无法删除该分组".into());
    }
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    ensure_library_group_schema(&conn)?;
    conn.execute(
        "UPDATE library_items SET group_id = '' WHERE platform = ?1 AND group_id = ?2",
        params![plat, id],
    )
    .map_err(|e| e.to_string())?;
    let n = conn
        .execute(
            "DELETE FROM library_groups WHERE id = ?1 AND platform = ?2",
            params![id, plat],
        )
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "deleted": n }))
}

#[tauri::command]
pub fn library_group_reorder(
    state: State<'_, LibraryState>,
    platform: String,
    ids: Vec<String>,
) -> Result<(), String> {
    let plat = platform.trim().to_lowercase();
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    ensure_library_group_schema(&conn)?;
    for (i, id) in ids.iter().enumerate() {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        conn.execute(
            "UPDATE library_groups SET sort_order = ?3, updated_at = datetime('now') WHERE id = ?1 AND platform = ?2",
            params![id, plat, i as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn library_move_to_group(
    state: State<'_, LibraryState>,
    platform: String,
    item_ids: Vec<String>,
    group_id: String,
) -> Result<Value, String> {
    let plat = platform.trim().to_lowercase();
    let gid = group_id.trim().to_string();
    if !gid.is_empty() && gid != ungrouped_sys_id(&plat) {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        ensure_library_group_schema(&conn)?;
        let exists: Option<String> = conn
            .query_row(
                "SELECT id FROM library_groups WHERE id = ?1 AND platform = ?2",
                params![gid, plat],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if exists.is_none() {
            return Err("目标分组不存在".into());
        }
        drop(conn);
    }
    let target = if gid == ungrouped_sys_id(&plat) {
        String::new()
    } else {
        gid
    };
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    ensure_library_group_schema(&conn)?;
    let mut moved = 0i64;
    for item_id in item_ids {
        let item_id = item_id.trim();
        if item_id.is_empty() {
            continue;
        }
        let n = conn
            .execute(
                "UPDATE library_items SET group_id = ?3 WHERE platform = ?1 AND item_id = ?2",
                params![plat, item_id, target],
            )
            .map_err(|e| e.to_string())?;
        moved += n as i64;
    }
    Ok(serde_json::json!({ "ok": true, "moved": moved }))
}

#[tauri::command]
pub fn library_set_analysis(
    state: State<'_, LibraryState>,
    platform: String,
    scope: String,
    id: String,
    analysis_json: String,
) -> Result<(), String> {
    let plat = platform.trim().to_lowercase();
    let scope = scope.trim().to_lowercase();
    let id = id.trim().to_string();
    let analysis_json = analysis_json.trim().to_string();
    if analysis_json.is_empty() {
        return Err("分析结果为空".into());
    }
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    ensure_library_group_schema(&conn)?;

    match scope.as_str() {
        "item" => {
            if id.is_empty() {
                return Err("缺少笔记 id".into());
            }
            let n = conn
                .execute(
                    r#"
                    UPDATE library_items
                    SET analysis_json = ?3, analyzed_at = datetime('now')
                    WHERE platform = ?1 AND item_id = ?2
                    "#,
                    params![plat, id, analysis_json],
                )
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("内容不存在".into());
            }
        }
        "group" => {
            let gid = if id.is_empty() {
                ungrouped_sys_id(&plat)
            } else {
                id.clone()
            };
            if gid == ungrouped_sys_id(&plat) {
                conn.execute(
                    r#"
                    INSERT INTO library_groups (id, platform, name, sort_order, analysis_json, analyzed_at, created_at, updated_at)
                    VALUES (?1, ?2, '__ungrouped__', -1, ?3, datetime('now'), datetime('now'), datetime('now'))
                    ON CONFLICT(id) DO UPDATE SET
                        analysis_json = excluded.analysis_json,
                        analyzed_at = datetime('now'),
                        updated_at = datetime('now')
                    "#,
                    params![gid, plat, analysis_json],
                )
                .map_err(|e| e.to_string())?;
            } else {
                let n = conn
                    .execute(
                        r#"
                        UPDATE library_groups
                        SET analysis_json = ?3, analyzed_at = datetime('now'), updated_at = datetime('now')
                        WHERE id = ?1 AND platform = ?2
                        "#,
                        params![gid, plat, analysis_json],
                    )
                    .map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("分组不存在".into());
                }
            }
        }
        _ => return Err(format!("未知 scope: {scope}")),
    }
    Ok(())
}
