use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};

use crate::library::LibraryState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaAsset {
    pub id: String,
    pub kind: String,
    pub file_name: String,
    pub rel_path: String,
    pub abs_path: String,
    pub mime: String,
    pub size_bytes: i64,
    pub source: String,
    pub prompt: String,
    pub model: String,
    pub draft_id: String,
    pub platform: String,
    pub folder_id: String,
    pub sort_order: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaFolder {
    pub id: String,
    pub name: String,
    pub parent_id: String,
    pub sort_order: i64,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestAssetBody {
    /// http(s) / data URL / local absolute path
    pub source_url: Option<String>,
    pub bytes_base64: Option<String>,
    pub kind: Option<String>,
    pub mime: Option<String>,
    pub source: Option<String>,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub draft_id: Option<String>,
    pub platform: Option<String>,
    pub file_name: Option<String>,
    pub folder_id: Option<String>,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn new_id() -> String {
    format!("as_{}", now_ms())
}

pub fn assets_root(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?
        .join("ai-assets");
    fs::create_dir_all(dir.join("images")).map_err(|e| format!("mkdir images: {e}"))?;
    fs::create_dir_all(dir.join("videos")).map_err(|e| format!("mkdir videos: {e}"))?;
    Ok(dir)
}

pub fn ensure_assets_table(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS media_assets (
            id TEXT PRIMARY KEY NOT NULL,
            kind TEXT NOT NULL DEFAULT 'image',
            file_name TEXT NOT NULL DEFAULT '',
            rel_path TEXT NOT NULL DEFAULT '',
            abs_path TEXT NOT NULL DEFAULT '',
            mime TEXT NOT NULL DEFAULT '',
            size_bytes INTEGER NOT NULL DEFAULT 0,
            source TEXT NOT NULL DEFAULT '',
            prompt TEXT NOT NULL DEFAULT '',
            model TEXT NOT NULL DEFAULT '',
            draft_id TEXT NOT NULL DEFAULT '',
            platform TEXT NOT NULL DEFAULT '',
            folder_id TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS media_folders (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL DEFAULT '',
            parent_id TEXT NOT NULL DEFAULT '',
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_media_assets_kind_created
            ON media_assets(kind, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_media_assets_draft
            ON media_assets(draft_id);
        CREATE INDEX IF NOT EXISTS idx_media_folders_parent
            ON media_folders(parent_id, name);
        "#,
    )
    .map_err(|e| format!("media_assets migrate: {e}"))?;
    // legacy DBs created before folder_id — must run before folder index
    let _ = conn.execute(
        "ALTER TABLE media_assets ADD COLUMN folder_id TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE media_assets ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE media_folders ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0",
        [],
    );
    // one-time-ish backfill for legacy zero orders
    let _ = conn.execute(
        "UPDATE media_assets
         SET sort_order = CAST(strftime('%s', created_at) AS INTEGER)
         WHERE sort_order = 0",
        [],
    );
    let _ = conn.execute(
        "UPDATE media_folders
         SET sort_order = CAST(strftime('%s', created_at) AS INTEGER)
         WHERE sort_order = 0",
        [],
    );
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_media_assets_folder_sort
            ON media_assets(folder_id, sort_order ASC, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_media_folders_parent_sort
            ON media_folders(parent_id, sort_order ASC, name COLLATE NOCASE);
        "#,
    )
    .map_err(|e| format!("media sort index: {e}"))?;
    Ok(())
}

fn map_asset(row: &rusqlite::Row<'_>) -> rusqlite::Result<MediaAsset> {
    Ok(MediaAsset {
        id: row.get(0)?,
        kind: row.get(1)?,
        file_name: row.get(2)?,
        rel_path: row.get(3)?,
        abs_path: row.get(4)?,
        mime: row.get(5)?,
        size_bytes: row.get(6)?,
        source: row.get(7)?,
        prompt: row.get(8)?,
        model: row.get(9)?,
        draft_id: row.get(10)?,
        platform: row.get(11)?,
        folder_id: row.get(12).unwrap_or_default(),
        sort_order: row.get(13).unwrap_or(0),
        created_at: row.get(14)?,
    })
}

const ASSET_SELECT: &str = "SELECT id, kind, file_name, rel_path, abs_path, mime, size_bytes, source, prompt, model, draft_id, platform, COALESCE(folder_id,''), COALESCE(sort_order,0), created_at FROM media_assets";

fn guess_kind(mime: &str, hint: &str) -> String {
    let h = hint.trim().to_ascii_lowercase();
    if h == "video" || mime.starts_with("video/") {
        return "video".into();
    }
    "image".into()
}

fn ext_for(mime: &str, kind: &str) -> &'static str {
    let m = mime.to_ascii_lowercase();
    if m.contains("jpeg") || m.contains("jpg") {
        return "jpg";
    }
    if m.contains("webp") {
        return "webp";
    }
    if m.contains("gif") {
        return "gif";
    }
    if m.contains("png") {
        return "png";
    }
    if m.contains("mp4") {
        return "mp4";
    }
    if m.contains("webm") {
        return "webm";
    }
    if m.contains("mov") {
        return "mov";
    }
    if kind == "video" {
        "mp4"
    } else {
        "png"
    }
}

fn decode_source(body: &IngestAssetBody) -> Result<(Vec<u8>, String), String> {
    if let Some(b64) = body
        .bytes_base64
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let raw = if let Some((_, data)) = b64.split_once(',') {
            data
        } else {
            b64
        };
        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            raw,
        )
        .map_err(|e| format!("base64 解码失败: {e}"))?;
        let mime = body
            .mime
            .clone()
            .unwrap_or_else(|| "image/png".into());
        return Ok((bytes, mime));
    }

    let src = body
        .source_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "缺少 sourceUrl 或 bytesBase64".to_string())?;

    if src.starts_with("data:") {
        let m = regex_data_url(src)?;
        return Ok(m);
    }

    if src.starts_with("http://") || src.starts_with("https://") {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(15))
            .timeout_read(std::time::Duration::from_secs(120))
            .build();
        let res = agent
            .get(src)
            .call()
            .map_err(|e| format!("下载素材失败: {e}"))?;
        let mime = res
            .header("content-type")
            .unwrap_or("image/png")
            .split(';')
            .next()
            .unwrap_or("image/png")
            .trim()
            .to_string();
        let mut bytes = Vec::new();
        res.into_reader()
            .read_to_end(&mut bytes)
            .map_err(|e| format!("读取素材失败: {e}"))?;
        return Ok((bytes, mime));
    }

    // local path
    let path = Path::new(src);
    if !path.is_file() {
        return Err(format!("本地文件不存在: {src}"));
    }
    let bytes = fs::read(path).map_err(|e| format!("读取本地文件失败: {e}"))?;
    let mime = body.mime.clone().unwrap_or_else(|| {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg".into(),
            "webp" => "image/webp".into(),
            "gif" => "image/gif".into(),
            "mp4" => "video/mp4".into(),
            "webm" => "video/webm".into(),
            "mov" => "video/quicktime".into(),
            _ => "image/png".into(),
        }
    });
    Ok((bytes, mime))
}

fn regex_data_url(src: &str) -> Result<(Vec<u8>, String), String> {
    let rest = src.strip_prefix("data:").ok_or("无效 data URL")?;
    let (meta, data) = rest.split_once(',').ok_or("无效 data URL")?;
    if !meta.contains(";base64") {
        return Err("仅支持 base64 data URL".into());
    }
    let mime = meta
        .split(';')
        .next()
        .unwrap_or("image/png")
        .trim()
        .to_string();
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)
        .map_err(|e| format!("data URL 解码失败: {e}"))?;
    Ok((bytes, mime))
}

fn next_sort_order(conn: &rusqlite::Connection, folder_id: &str) -> i64 {
    conn.query_row(
        "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM media_assets WHERE COALESCE(folder_id,'') = ?1",
        params![folder_id],
        |r| r.get(0),
    )
    .unwrap_or(1)
}

pub fn insert_asset_row(conn: &rusqlite::Connection, asset: &MediaAsset) -> Result<(), String> {
    let sort_order = if asset.sort_order > 0 {
        asset.sort_order
    } else {
        next_sort_order(conn, &asset.folder_id)
    };
    conn.execute(
        "INSERT INTO media_assets
         (id, kind, file_name, rel_path, abs_path, mime, size_bytes, source, prompt, model, draft_id, platform, folder_id, sort_order, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14, datetime('now'))",
        params![
            asset.id,
            asset.kind,
            asset.file_name,
            asset.rel_path,
            asset.abs_path,
            asset.mime,
            asset.size_bytes,
            asset.source,
            asset.prompt,
            asset.model,
            asset.draft_id,
            asset.platform,
            asset.folder_id,
            sort_order,
        ],
    )
    .map_err(|e| format!("写入素材索引失败: {e}"))?;
    Ok(())
}

pub fn fetch_asset_row(conn: &rusqlite::Connection, id: &str) -> Result<MediaAsset, String> {
    conn.query_row(
        &format!("{ASSET_SELECT} WHERE id = ?1"),
        params![id],
        map_asset,
    )
    .map_err(|e| format!("素材不存在: {e}"))
}

fn insert_asset(conn: &rusqlite::Connection, asset: &MediaAsset) -> Result<(), String> {
    insert_asset_row(conn, asset)
}

fn fetch_asset(conn: &rusqlite::Connection, id: &str) -> Result<MediaAsset, String> {
    fetch_asset_row(conn, id)
}

#[tauri::command]
pub fn asset_dir_info(app: AppHandle) -> Result<Value, String> {
    let root = assets_root(&app)?;
    Ok(serde_json::json!({
        "root": root.to_string_lossy(),
        "images": root.join("images").to_string_lossy(),
        "videos": root.join("videos").to_string_lossy(),
    }))
}

#[tauri::command]
pub fn asset_list(
    db: State<'_, LibraryState>,
    kind: Option<String>,
    query: Option<String>,
    folder_id: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<MediaAsset>, String> {
    let lim = limit.unwrap_or(100).clamp(1, 500);
    let off = offset.unwrap_or(0).max(0);
    let kind = kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all")
        .map(|s| s.to_ascii_lowercase());
    let q = query
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!("%{s}%"));
    let folder_filter = folder_id.is_some();
    let folder = folder_id.unwrap_or_default();

    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut sql = format!("{ASSET_SELECT} WHERE 1=1");
    if kind.is_some() {
        sql.push_str(" AND kind = ?");
    }
    if q.is_some() {
        sql.push_str(
            " AND (prompt LIKE ? COLLATE NOCASE OR file_name LIKE ? COLLATE NOCASE OR model LIKE ? COLLATE NOCASE)",
        );
    }
    if folder_filter {
        sql.push_str(" AND COALESCE(folder_id,'') = ?");
    }
    sql.push_str(" ORDER BY sort_order ASC, created_at DESC LIMIT ? OFFSET ?");

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(k) = &kind {
        vals.push(Box::new(k.clone()));
    }
    if let Some(qq) = &q {
        vals.push(Box::new(qq.clone()));
        vals.push(Box::new(qq.clone()));
        vals.push(Box::new(qq.clone()));
    }
    if folder_filter {
        vals.push(Box::new(folder));
    }
    vals.push(Box::new(lim));
    vals.push(Box::new(off));
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(params_ref.as_slice(), map_asset)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn asset_get(db: State<'_, LibraryState>, id: String) -> Result<Option<MediaAsset>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        &format!("{ASSET_SELECT} WHERE id = ?1"),
        params![id.trim()],
        map_asset,
    )
    .optional()
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn asset_ingest(
    app: AppHandle,
    db: State<'_, LibraryState>,
    body: IngestAssetBody,
) -> Result<MediaAsset, String> {
    let root = assets_root(&app)?;
    let (bytes, mime) = decode_source(&body)?;
    if bytes.is_empty() {
        return Err("素材内容为空".into());
    }
    let kind = guess_kind(&mime, body.kind.as_deref().unwrap_or(""));
    let sub = if kind == "video" { "videos" } else { "images" };
    let ext = ext_for(&mime, &kind);
    let id = new_id();
    let file_name = body
        .file_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{id}.{ext}"));
    let rel_path = format!("{sub}/{file_name}");
    let abs_path = root.join(&rel_path);
    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    fs::write(&abs_path, &bytes).map_err(|e| format!("写入素材失败: {e}"))?;

    let asset = MediaAsset {
        id: id.clone(),
        kind,
        file_name,
        rel_path,
        abs_path: abs_path.to_string_lossy().to_string(),
        mime,
        size_bytes: bytes.len() as i64,
        source: body.source.unwrap_or_else(|| "ai".into()),
        prompt: body.prompt.unwrap_or_default(),
        model: body.model.unwrap_or_default(),
        draft_id: body.draft_id.unwrap_or_default(),
        platform: body
            .platform
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase(),
        folder_id: body.folder_id.unwrap_or_default(),
        sort_order: 0,
        created_at: String::new(),
    };

    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    insert_asset(&conn, &asset)?;
    fetch_asset(&conn, &id)
}

fn map_folder(row: &rusqlite::Row<'_>) -> rusqlite::Result<MediaFolder> {
    Ok(MediaFolder {
        id: row.get(0)?,
        name: row.get(1)?,
        parent_id: row.get(2)?,
        sort_order: row.get(3).unwrap_or(0),
        created_at: row.get(4).unwrap_or_default(),
    })
}

fn new_folder_id() -> String {
    format!("fd_{}", now_ms())
}

#[tauri::command]
pub fn media_folder_list(
    db: State<'_, LibraryState>,
    parent_id: Option<String>,
    query: Option<String>,
) -> Result<Vec<MediaFolder>, String> {
    let parent = parent_id.unwrap_or_default();
    let q = query
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!("%{s}%"));
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if let Some(qq) = &q {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, parent_id, COALESCE(sort_order,0), created_at FROM media_folders
                 WHERE parent_id = ?1 AND name LIKE ?2 COLLATE NOCASE
                 ORDER BY sort_order ASC, name COLLATE NOCASE ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![parent, qq], map_folder)
            .map_err(|e| e.to_string())?;
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, parent_id, COALESCE(sort_order,0), created_at FROM media_folders
                 WHERE parent_id = ?1
                 ORDER BY sort_order ASC, name COLLATE NOCASE ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![parent], map_folder)
            .map_err(|e| e.to_string())?;
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
    }
    Ok(out)
}

#[tauri::command]
pub fn media_folder_create(
    db: State<'_, LibraryState>,
    name: String,
    parent_id: Option<String>,
) -> Result<MediaFolder, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("文件夹名称不能为空".into());
    }
    let parent = parent_id.unwrap_or_default();
    let id = new_folder_id();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM media_folders WHERE parent_id = ?1 AND name = ?2 COLLATE NOCASE",
            params![parent, name],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if exists > 0 {
        return Err("同级已存在同名文件夹".into());
    }
    let sort_order: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM media_folders WHERE parent_id = ?1",
            params![parent],
            |r| r.get(0),
        )
        .unwrap_or(1);
    conn.execute(
        "INSERT INTO media_folders (id, name, parent_id, sort_order, created_at)
         VALUES (?1,?2,?3,?4, datetime('now'))",
        params![id, name, parent, sort_order],
    )
    .map_err(|e| format!("创建文件夹失败: {e}"))?;
    conn.query_row(
        "SELECT id, name, parent_id, COALESCE(sort_order,0), created_at FROM media_folders WHERE id = ?1",
        params![id],
        map_folder,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn media_folder_rename(
    db: State<'_, LibraryState>,
    id: String,
    name: String,
) -> Result<MediaFolder, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("文件夹名称不能为空".into());
    }
    let id = id.trim().to_string();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let parent: String = conn
        .query_row(
            "SELECT parent_id FROM media_folders WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|_| "文件夹不存在".to_string())?;
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM media_folders WHERE parent_id = ?1 AND name = ?2 COLLATE NOCASE AND id != ?3",
            params![parent, name, id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if exists > 0 {
        return Err("同级已存在同名文件夹".into());
    }
    conn.execute(
        "UPDATE media_folders SET name = ?2 WHERE id = ?1",
        params![id, name],
    )
    .map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT id, name, parent_id, COALESCE(sort_order,0), created_at FROM media_folders WHERE id = ?1",
        params![id],
        map_folder,
    )
    .map_err(|e| e.to_string())
}

fn collect_folder_subtree(conn: &rusqlite::Connection, root_id: &str) -> Result<Vec<String>, String> {
    let mut out = vec![root_id.to_string()];
    let mut i = 0;
    while i < out.len() {
        let cur = out[i].clone();
        let mut stmt = conn
            .prepare("SELECT id FROM media_folders WHERE parent_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![cur], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        i += 1;
    }
    Ok(out)
}

#[tauri::command]
pub fn media_folder_delete(db: State<'_, LibraryState>, id: String) -> Result<Value, String> {
    let id = id.trim().to_string();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let ids = collect_folder_subtree(&conn, &id)?;
    for fid in &ids {
        conn.execute(
            "UPDATE media_assets SET folder_id = '' WHERE folder_id = ?1",
            params![fid],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE image_jobs SET folder_id = '' WHERE folder_id = ?1",
            params![fid],
        )
        .ok();
        conn.execute("DELETE FROM media_folders WHERE id = ?1", params![fid])
            .map_err(|e| e.to_string())?;
    }
    Ok(serde_json::json!({ "ok": true, "deleted": ids.len() }))
}

#[tauri::command]
pub fn media_folder_get(
    db: State<'_, LibraryState>,
    id: String,
) -> Result<Option<MediaFolder>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT id, name, parent_id, COALESCE(sort_order,0), created_at FROM media_folders WHERE id = ?1",
        params![id.trim()],
        map_folder,
    )
    .optional()
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn asset_move(
    db: State<'_, LibraryState>,
    ids: Vec<String>,
    folder_id: Option<String>,
) -> Result<Value, String> {
    let folder = folder_id.unwrap_or_default();
    if !folder.is_empty() {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let ok: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM media_folders WHERE id = ?1",
                params![folder],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if ok == 0 {
            return Err("目标文件夹不存在".into());
        }
    }
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut n = 0;
    for id in ids {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        let order = next_sort_order(&conn, &folder);
        n += conn
            .execute(
                "UPDATE media_assets SET folder_id = ?2, sort_order = ?3 WHERE id = ?1",
                params![id, folder, order],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(serde_json::json!({ "ok": true, "moved": n }))
}

#[tauri::command]
pub fn asset_reorder(db: State<'_, LibraryState>, ids: Vec<String>) -> Result<Value, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut n = 0i64;
    for (i, id) in ids.into_iter().enumerate() {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        n += conn
            .execute(
                "UPDATE media_assets SET sort_order = ?2 WHERE id = ?1",
                params![id, i as i64 + 1],
            )
            .map_err(|e| e.to_string())? as i64;
    }
    Ok(serde_json::json!({ "ok": true, "updated": n }))
}

#[tauri::command]
pub fn media_folder_reorder(
    db: State<'_, LibraryState>,
    ids: Vec<String>,
) -> Result<Value, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut n = 0i64;
    for (i, id) in ids.into_iter().enumerate() {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        n += conn
            .execute(
                "UPDATE media_folders SET sort_order = ?2 WHERE id = ?1",
                params![id, i as i64 + 1],
            )
            .map_err(|e| e.to_string())? as i64;
    }
    Ok(serde_json::json!({ "ok": true, "updated": n }))
}

#[tauri::command]
pub fn media_folder_move(
    db: State<'_, LibraryState>,
    id: String,
    parent_id: Option<String>,
) -> Result<MediaFolder, String> {
    let id = id.trim().to_string();
    if id.is_empty() {
        return Err("文件夹不存在".into());
    }
    let parent = parent_id.unwrap_or_default();
    if parent == id {
        return Err("不能移入自身".into());
    }
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM media_folders WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if exists == 0 {
        return Err("文件夹不存在".into());
    }
    if !parent.is_empty() {
        let pok: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM media_folders WHERE id = ?1",
                params![parent],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if pok == 0 {
            return Err("目标文件夹不存在".into());
        }
        let subtree = collect_folder_subtree(&conn, &id)?;
        if subtree.iter().any(|x| x == &parent) {
            return Err("不能移入自己的子文件夹".into());
        }
    }
    let name: String = conn
        .query_row(
            "SELECT name FROM media_folders WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|_| "文件夹不存在".to_string())?;
    let clash: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM media_folders
             WHERE parent_id = ?1 AND name = ?2 COLLATE NOCASE AND id != ?3",
            params![parent, name, id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if clash > 0 {
        return Err("目标位置已有同名文件夹".into());
    }
    let sort_order: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM media_folders WHERE parent_id = ?1",
            params![parent],
            |r| r.get(0),
        )
        .unwrap_or(1);
    conn.execute(
        "UPDATE media_folders SET parent_id = ?2, sort_order = ?3 WHERE id = ?1",
        params![id, parent, sort_order],
    )
    .map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT id, name, parent_id, COALESCE(sort_order,0), created_at FROM media_folders WHERE id = ?1",
        params![id],
        map_folder,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn asset_delete(db: State<'_, LibraryState>, id: String) -> Result<Value, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let asset = fetch_asset(&conn, id.trim())?;
    let _ = fs::remove_file(&asset.abs_path);
    let n = conn
        .execute("DELETE FROM media_assets WHERE id = ?1", params![id.trim()])
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "deleted": n }))
}

fn sanitize_file_name(raw: &str) -> String {
    let mut s = raw.trim().replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    while s.starts_with('.') {
        s = s[1..].to_string();
    }
    s.trim().to_string()
}

fn split_ext(name: &str) -> (String, String) {
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|x| x.to_str())
        .unwrap_or(name)
        .to_string();
    let ext = path
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_string();
    (stem, ext)
}

#[tauri::command]
pub fn asset_rename(
    app: AppHandle,
    db: State<'_, LibraryState>,
    id: String,
    name: String,
) -> Result<MediaAsset, String> {
    let id = id.trim().to_string();
    let mut name = sanitize_file_name(&name);
    if name.is_empty() {
        return Err("名称不能为空".into());
    }
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let asset = fetch_asset(&conn, &id)?;
    let (_old_stem, old_ext) = split_ext(&asset.file_name);
    let (new_stem, new_ext) = split_ext(&name);
    if new_stem.is_empty() {
        return Err("名称不能为空".into());
    }
    // 未写扩展名时保留原扩展名
    let final_name = if new_ext.is_empty() && !old_ext.is_empty() {
        format!("{new_stem}.{old_ext}")
    } else if new_ext.is_empty() {
        new_stem
    } else {
        format!("{new_stem}.{new_ext}")
    };
    name = final_name;

    if name == asset.file_name {
        return Ok(asset);
    }

    let sub = if asset.kind == "video" { "videos" } else { "images" };
    let root = assets_root(&app)?;
    let new_rel = format!("{sub}/{name}");
    let new_abs = root.join(&new_rel);
    if new_abs.exists() {
        return Err("同名文件已存在".into());
    }
    if let Some(parent) = new_abs.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let old_abs = PathBuf::from(&asset.abs_path);
    if old_abs.exists() {
        fs::rename(&old_abs, &new_abs).map_err(|e| format!("重命名文件失败: {e}"))?;
    }

    conn.execute(
        "UPDATE media_assets SET file_name = ?2, rel_path = ?3, abs_path = ?4 WHERE id = ?1",
        params![
            id,
            name,
            new_rel,
            new_abs.to_string_lossy().to_string()
        ],
    )
    .map_err(|e| e.to_string())?;
    fetch_asset(&conn, &id)
}

#[tauri::command]
pub fn asset_reveal(app: AppHandle, id: String, db: State<'_, LibraryState>) -> Result<Value, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let asset = fetch_asset(&conn, id.trim())?;
    drop(conn);
    let path = PathBuf::from(&asset.abs_path);
    if !path.exists() {
        return Err("文件已丢失".into());
    }
    // best-effort open containing folder via opener plugin path
    let _ = app;
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.to_string_lossy()))
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(parent) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
        }
    }
    Ok(serde_json::json!({ "ok": true, "path": asset.abs_path }))
}
