use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, State};

use crate::library::LibraryState;
use crate::media_cache::{cache_note_media, remove_note_media};

const MAX_COMPETITORS: i64 = 20;
const MAX_WATCHED: i64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompetitorRow {
    pub id: String,
    pub note_url: String,
    pub note_id: String,
    pub title: String,
    pub author_name: String,
    pub cover_url: String,
    pub cover_local_path: String,
    pub likes: i64,
    pub collects: i64,
    pub comments: i64,
    pub shares: i64,
    pub interval_minutes: i64,
    pub next_run_at: String,
    pub last_refreshed_at: String,
    pub last_error: String,
    pub status: String,
    pub analysis_score: i64,
    pub analysis_grade: String,
    pub opportunity_count: i64,
    pub keywords_json: String,
    pub analysis_json: String,
    pub chat_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRow {
    pub id: i64,
    pub competitor_id: String,
    pub likes: i64,
    pub collects: i64,
    pub comments: i64,
    pub shares: i64,
    pub engagement: i64,
    pub captured_at: String,
    /// live=真实刷新；backfill=为凑满近 N 天展示而补齐
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchedUserRow {
    pub id: String,
    pub external_user_id: String,
    pub nickname: String,
    pub avatar_url: String,
    pub profile_url: String,
    pub note_count: i64,
    pub last_synced_at: String,
    pub interval_minutes: i64,
    pub next_run_at: String,
    pub status: String,
    pub last_error: String,
    pub latest_notes_json: String,
    pub unread_events: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchedEventRow {
    pub id: String,
    pub watched_user_id: String,
    pub event_type: String,
    pub title: String,
    pub note_url: String,
    pub note_id: String,
    pub is_read: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityRow {
    pub id: String,
    pub competitor_id: String,
    pub keyword: String,
    pub comment_text: String,
    pub author_nickname: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentRow {
    pub id: String,
    pub competitor_id: String,
    pub user_name: String,
    pub content: String,
    pub like_count: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshLogRow {
    pub id: i64,
    pub target_id: String,
    pub status: String,
    pub message: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddCompetitorBody {
    pub note_url: String,
    pub note_id: Option<String>,
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub cover_url: Option<String>,
    pub likes: Option<i64>,
    pub collects: Option<i64>,
    pub comments: Option<i64>,
    pub shares: Option<i64>,
    pub interval_minutes: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshCompetitorBody {
    pub id: String,
    pub note_url: Option<String>,
    pub note_id: Option<String>,
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub cover_url: Option<String>,
    pub likes: Option<i64>,
    pub collects: Option<i64>,
    pub comments: Option<i64>,
    pub shares: Option<i64>,
    pub analysis_json: Option<String>,
    pub analysis_score: Option<i64>,
    pub analysis_grade: Option<String>,
    pub last_error: Option<String>,
    pub comments_json: Option<String>,
    /// 执行记录触发方式，如「手动刷新」「后台监控」
    pub trigger: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddWatchedBody {
    pub profile_url: String,
    pub external_user_id: Option<String>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshWatchedBody {
    pub id: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub note_count: Option<i64>,
    pub latest_notes_json: Option<String>,
    pub last_error: Option<String>,
    pub new_events: Option<Vec<NewEventBody>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewEventBody {
    pub event_type: Option<String>,
    pub title: Option<String>,
    pub note_url: Option<String>,
    pub note_id: Option<String>,
}

fn new_id(prefix: &str) -> String {
    format!(
        "{prefix}_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    )
}

fn has_column(conn: &Connection, table: &str, col: &str) -> bool {
    let sql = format!("PRAGMA table_info({table})");
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return false;
    };
    let Ok(mapped) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
        return false;
    };
    let cols: Vec<String> = mapped.filter_map(|r| r.ok()).collect();
    cols.iter().any(|c| c == col)
}

pub fn ensure_competitor_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS competitors (
            id TEXT PRIMARY KEY NOT NULL,
            note_url TEXT NOT NULL,
            note_id TEXT NOT NULL DEFAULT '',
            title TEXT NOT NULL DEFAULT '',
            author_name TEXT NOT NULL DEFAULT '',
            cover_url TEXT NOT NULL DEFAULT '',
            likes INTEGER NOT NULL DEFAULT 0,
            collects INTEGER NOT NULL DEFAULT 0,
            comments INTEGER NOT NULL DEFAULT 0,
            shares INTEGER NOT NULL DEFAULT 0,
            interval_minutes INTEGER NOT NULL DEFAULT 60,
            next_run_at TEXT NOT NULL DEFAULT '',
            last_refreshed_at TEXT NOT NULL DEFAULT '',
            last_error TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'active',
            analysis_score INTEGER NOT NULL DEFAULT 0,
            analysis_grade TEXT NOT NULL DEFAULT '',
            opportunity_count INTEGER NOT NULL DEFAULT 0,
            keywords_json TEXT NOT NULL DEFAULT '[]',
            analysis_json TEXT NOT NULL DEFAULT '',
            chat_json TEXT NOT NULL DEFAULT '{"messages":[]}',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(note_url)
        );
        CREATE TABLE IF NOT EXISTS competitor_snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            competitor_id TEXT NOT NULL,
            likes INTEGER NOT NULL DEFAULT 0,
            collects INTEGER NOT NULL DEFAULT 0,
            comments INTEGER NOT NULL DEFAULT 0,
            shares INTEGER NOT NULL DEFAULT 0,
            engagement INTEGER NOT NULL DEFAULT 0,
            captured_at TEXT NOT NULL DEFAULT (datetime('now')),
            source TEXT NOT NULL DEFAULT 'live'
        );
        CREATE TABLE IF NOT EXISTS watched_users (
            id TEXT PRIMARY KEY NOT NULL,
            external_user_id TEXT NOT NULL DEFAULT '',
            nickname TEXT NOT NULL DEFAULT '',
            avatar_url TEXT NOT NULL DEFAULT '',
            profile_url TEXT NOT NULL DEFAULT '',
            note_count INTEGER NOT NULL DEFAULT 0,
            last_synced_at TEXT NOT NULL DEFAULT '',
            interval_minutes INTEGER NOT NULL DEFAULT 360,
            next_run_at TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'active',
            last_error TEXT NOT NULL DEFAULT '',
            latest_notes_json TEXT NOT NULL DEFAULT '[]',
            unread_events INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(profile_url)
        );
        CREATE TABLE IF NOT EXISTS watched_user_events (
            id TEXT PRIMARY KEY NOT NULL,
            watched_user_id TEXT NOT NULL,
            event_type TEXT NOT NULL DEFAULT 'new_note',
            title TEXT NOT NULL DEFAULT '',
            note_url TEXT NOT NULL DEFAULT '',
            note_id TEXT NOT NULL DEFAULT '',
            is_read INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS competitor_comments (
            id TEXT PRIMARY KEY NOT NULL,
            competitor_id TEXT NOT NULL,
            user_name TEXT NOT NULL DEFAULT '',
            content TEXT NOT NULL DEFAULT '',
            like_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS opportunities (
            id TEXT PRIMARY KEY NOT NULL,
            competitor_id TEXT NOT NULL,
            keyword TEXT NOT NULL DEFAULT '',
            comment_text TEXT NOT NULL DEFAULT '',
            author_nickname TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'open',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS refresh_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            target_id TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT '',
            message TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS app_kv (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL DEFAULT '',
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        "#,
    )
    .map_err(|e| format!("competitor schema: {e}"))?;

    let alters = [
        ("competitors", "last_refreshed_at", "TEXT NOT NULL DEFAULT ''"),
        ("competitors", "last_error", "TEXT NOT NULL DEFAULT ''"),
        ("competitors", "status", "TEXT NOT NULL DEFAULT 'active'"),
        ("competitors", "analysis_score", "INTEGER NOT NULL DEFAULT 0"),
        ("competitors", "analysis_grade", "TEXT NOT NULL DEFAULT ''"),
        ("competitors", "opportunity_count", "INTEGER NOT NULL DEFAULT 0"),
        ("competitors", "keywords_json", "TEXT NOT NULL DEFAULT '[]'"),
        ("competitors", "cover_local_path", "TEXT NOT NULL DEFAULT ''"),
        (
            "competitor_snapshots",
            "source",
            "TEXT NOT NULL DEFAULT 'live'",
        ),
    ];
    for (table, col, typ) in alters {
        if !has_column(conn, table, col) {
            conn.execute(
                &format!("ALTER TABLE {table} ADD COLUMN {col} {typ}"),
                [],
            )
            .map_err(|e| format!("alter {table}.{col}: {e}"))?;
        }
    }
    migrate_next_run_to_local(conn)?;
    migrate_stagger_next_runs(conn)?;
    Ok(())
}

fn map_comp(row: &rusqlite::Row<'_>) -> rusqlite::Result<CompetitorRow> {
    Ok(CompetitorRow {
        id: row.get(0)?,
        note_url: row.get(1)?,
        note_id: row.get(2)?,
        title: row.get(3)?,
        author_name: row.get(4)?,
        cover_url: row.get(5)?,
        cover_local_path: row.get(6)?,
        likes: row.get(7)?,
        collects: row.get(8)?,
        comments: row.get(9)?,
        shares: row.get(10)?,
        interval_minutes: row.get(11)?,
        next_run_at: row.get(12)?,
        last_refreshed_at: row.get(13)?,
        last_error: row.get(14)?,
        status: row.get(15)?,
        analysis_score: row.get(16)?,
        analysis_grade: row.get(17)?,
        opportunity_count: row.get(18)?,
        keywords_json: row.get(19)?,
        analysis_json: row.get(20)?,
        chat_json: row.get(21)?,
        created_at: row.get(22)?,
        updated_at: row.get(23)?,
    })
}

const COMP_SELECT: &str = "SELECT id, note_url, note_id, title, author_name, cover_url,
        COALESCE(cover_local_path,''), likes, collects, comments, shares,
        interval_minutes, next_run_at, last_refreshed_at, last_error, status, analysis_score, analysis_grade,
        opportunity_count, keywords_json, analysis_json, chat_json, created_at, updated_at
 FROM competitors";

/// 界面展示的 ±10 分钟抖动，避免多条笔记 next_run 落在同一秒
const NEXT_RUN_JITTER_MINUTES: i64 = 10;

fn jittered_delay_minutes(interval_minutes: i64, salt: &str) -> i64 {
    let base = interval_minutes.max(30);
    let span = NEXT_RUN_JITTER_MINUTES * 2 + 1; // -10..=+10
    let h = salt
        .bytes()
        .fold(0u64, |a, b| a.wrapping_mul(131).wrapping_add(u64::from(b)));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let j = ((h ^ nanos) % span as u64) as i64 - NEXT_RUN_JITTER_MINUTES;
    (base + j).max(20)
}

fn next_run_sql(interval_minutes: i64, salt: &str) -> String {
    let m = jittered_delay_minutes(interval_minutes, salt);
    // 用 localtime，与前端按本地墙钟判断到期一致
    format!("datetime('now', 'localtime', '+{m} minutes')")
}

/// 历史 next_run_at 按 UTC 写入时，在东八区会被误判为已到期并疯狂刷新；一次性拨回本地「现在+间隔」
fn migrate_next_run_to_local(conn: &Connection) -> Result<(), String> {
    let done: Option<String> = conn
        .query_row(
            "SELECT value FROM app_kv WHERE key = 'competitors_next_run_local_v1'",
            [],
            |r| r.get(0),
        )
        .ok();
    if done.as_deref() == Some("1") {
        return Ok(());
    }
    conn.execute(
        "UPDATE competitors SET next_run_at = datetime('now', 'localtime', '+' || CASE WHEN interval_minutes < 30 THEN 30 ELSE interval_minutes END || ' minutes')",
        [],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE watched_users SET next_run_at = datetime('now', 'localtime', '+' || CASE WHEN interval_minutes < 30 THEN 30 ELSE interval_minutes END || ' minutes')",
        [],
    )
    .ok();
    conn.execute(
        "INSERT INTO app_kv (key, value, updated_at) VALUES ('competitors_next_run_local_v1', '1', datetime('now', 'localtime'))
         ON CONFLICT(key) DO UPDATE SET value = '1', updated_at = excluded.updated_at",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// v1 迁移把所有笔记拨到同一 next_run；再按条错开（间隔 + 序号×3 分钟 + 抖动）
fn migrate_stagger_next_runs(conn: &Connection) -> Result<(), String> {
    let done: Option<String> = conn
        .query_row(
            "SELECT value FROM app_kv WHERE key = 'competitors_next_run_stagger_v1'",
            [],
            |r| r.get(0),
        )
        .ok();
    if done.as_deref() == Some("1") {
        return Ok(());
    }
    let mut stmt = conn
        .prepare("SELECT id, interval_minutes FROM competitors ORDER BY id")
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    for (i, (cid, interval)) in rows.into_iter().enumerate() {
        // 条与条之间至少错开数分钟，再叠 ±10 抖动，避免全部同一秒
        let mins = jittered_delay_minutes(interval.max(30) + (i as i64) * 5, &format!("{cid}:{i}"));
        conn.execute(
            &format!(
                "UPDATE competitors SET next_run_at = datetime('now', 'localtime', '+{mins} minutes') WHERE id = ?1"
            ),
            params![cid],
        )
        .map_err(|e| e.to_string())?;
    }
    conn.execute(
        "INSERT INTO app_kv (key, value, updated_at) VALUES ('competitors_next_run_stagger_v1', '1', datetime('now', 'localtime'))
         ON CONFLICT(key) DO UPDATE SET value = '1', updated_at = excluded.updated_at",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn competitor_list(db: State<'_, LibraryState>) -> Result<Vec<CompetitorRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let mut stmt = conn
        .prepare(&format!("{COMP_SELECT} ORDER BY updated_at DESC"))
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], map_comp).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn competitor_add(
    app: AppHandle,
    db: State<'_, LibraryState>,
    body: AddCompetitorBody,
) -> Result<CompetitorRow, String> {
    let url = body.note_url.trim().to_string();
    if url.is_empty() {
        return Err("笔记链接不能为空".into());
    }
    let cover_url = body.cover_url.clone().unwrap_or_default();
    let note_id = body.note_id.clone().unwrap_or_default();

    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM competitors", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if count >= MAX_COMPETITORS {
        return Err(format!("竞品监控最多 {MAX_COMPETITORS} 条"));
    }
    if let Ok(existing) = conn.query_row(
        &format!("{COMP_SELECT} WHERE note_url = ?1"),
        params![url],
        map_comp,
    ) {
        return Ok(existing);
    }
    drop(conn);

    let id = new_id("c");
    let media_key = if note_id.trim().is_empty() {
        id.clone()
    } else {
        note_id.clone()
    };
    let cover_local =
        cache_note_media(&app, "competitors", &media_key, &cover_url, &[]).cover_local;

    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let likes = body.likes.unwrap_or(0);
    let collects = body.collects.unwrap_or(0);
    let comments = body.comments.unwrap_or(0);
    let shares = body.shares.unwrap_or(0);
    let interval = body.interval_minutes.unwrap_or(60).max(30);
    let next = next_run_sql(interval, &id);

    conn.execute(
        &format!(
            "INSERT INTO competitors (
            id, note_url, note_id, title, author_name, cover_url, cover_local_path, likes, collects, comments, shares,
            interval_minutes, next_run_at, last_refreshed_at, last_error, status, analysis_score, analysis_grade,
            opportunity_count, keywords_json, analysis_json, chat_json, created_at, updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,{next},datetime('now','localtime'),'','active',0,'',0,'[]','','{{\"messages\":[]}}', datetime('now','localtime'), datetime('now','localtime'))"
        ),
        params![
            id,
            url,
            note_id,
            body.title.unwrap_or_default(),
            body.author_name.unwrap_or_default(),
            cover_url,
            cover_local,
            likes,
            collects,
            comments,
            shares,
            interval
        ],
    )
    .map_err(|e| e.to_string())?;

    let engagement = likes + collects + comments + shares;
    conn.execute(
        "INSERT INTO competitor_snapshots (competitor_id, likes, collects, comments, shares, engagement, captured_at, source)
         VALUES (?1,?2,?3,?4,?5,?6, datetime('now','localtime'), 'live')",
        params![id, likes, collects, comments, shares, engagement],
    )
    .map_err(|e| e.to_string())?;

    conn.query_row(&format!("{COMP_SELECT} WHERE id = ?1"), params![id], map_comp)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn competitor_delete(
    app: AppHandle,
    db: State<'_, LibraryState>,
    id: String,
) -> Result<Value, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let id = id.trim().to_string();
    let note_id: String = conn
        .query_row(
            "SELECT note_id FROM competitors WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap_or_default();
    conn.execute(
        "DELETE FROM competitor_snapshots WHERE competitor_id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM competitor_comments WHERE competitor_id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM opportunities WHERE competitor_id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    let n = conn
        .execute("DELETE FROM competitors WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    drop(conn);
    let media_key = if note_id.trim().is_empty() {
        id.clone()
    } else {
        note_id
    };
    remove_note_media(&app, "competitors", &media_key);
    remove_note_media(&app, "competitors", &id);
    Ok(serde_json::json!({ "ok": true, "deleted": n }))
}

#[tauri::command]
pub fn competitor_refresh(
    app: AppHandle,
    db: State<'_, LibraryState>,
    body: RefreshCompetitorBody,
) -> Result<CompetitorRow, String> {
    let id = body.id.trim().to_string();
    let cover_url = body.cover_url.clone().unwrap_or_default();
    let (note_id, interval): (String, i64) = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        ensure_competitor_schema(&conn)?;
        conn.query_row(
            "SELECT note_id, interval_minutes FROM competitors WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "竞品不存在".to_string())?
    };
    let media_key = if note_id.trim().is_empty() {
        id.clone()
    } else {
        note_id
    };
    let cover_local = if cover_url.trim().is_empty() {
        String::new()
    } else {
        cache_note_media(&app, "competitors", &media_key, &cover_url, &[]).cover_local
    };

    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let likes = body.likes.unwrap_or(0);
    let collects = body.collects.unwrap_or(0);
    let comments = body.comments.unwrap_or(0);
    let shares = body.shares.unwrap_or(0);
    let score = body.analysis_score.unwrap_or(0);
    let grade = body.analysis_grade.unwrap_or_default();
    let analysis = body.analysis_json.unwrap_or_default();
    let err = body.last_error.unwrap_or_default();
    let next = next_run_sql(interval, &id);

    conn.execute(
        &format!(
            "UPDATE competitors SET
            note_url=CASE WHEN length(trim(?2))>0 THEN ?2 ELSE note_url END,
            note_id=CASE WHEN length(trim(?3))>0 THEN ?3 ELSE note_id END,
            title=CASE WHEN length(trim(?4))>0 THEN ?4 ELSE title END,
            author_name=CASE WHEN length(trim(?5))>0 THEN ?5 ELSE author_name END,
            cover_url=CASE WHEN length(trim(?6))>0 THEN ?6 ELSE cover_url END,
            cover_local_path=CASE WHEN length(trim(?7))>0 THEN ?7 ELSE cover_local_path END,
            likes=?8, collects=?9, comments=?10, shares=?11,
            analysis_json=CASE WHEN length(trim(?12))>0 THEN ?12 ELSE analysis_json END,
            analysis_score=?13, analysis_grade=?14,
            last_error=?15,
            last_refreshed_at=datetime('now', 'localtime'),
            next_run_at={next},
            updated_at=datetime('now', 'localtime')
         WHERE id=?1"
        ),
        params![
            id,
            body.note_url.unwrap_or_default(),
            body.note_id.unwrap_or_default(),
            body.title.unwrap_or_default(),
            body.author_name.unwrap_or_default(),
            cover_url,
            cover_local,
            likes,
            collects,
            comments,
            shares,
            analysis,
            score,
            grade,
            err
        ],
    )
    .map_err(|e| e.to_string())?;

    let engagement = likes + collects + comments + shares;
    conn.execute(
        "INSERT INTO competitor_snapshots (competitor_id, likes, collects, comments, shares, engagement, captured_at, source)
         VALUES (?1,?2,?3,?4,?5,?6, datetime('now', 'localtime'), 'live')",
        params![id, likes, collects, comments, shares, engagement],
    )
    .map_err(|e| e.to_string())?;

    if let Some(cj) = body.comments_json.as_deref() {
        if let Ok(arr) = serde_json::from_str::<Vec<Value>>(cj) {
            let _ = conn.execute(
                "DELETE FROM competitor_comments WHERE competitor_id = ?1",
                params![id],
            );
            for (i, c) in arr.into_iter().take(80).enumerate() {
                let cid = c
                    .get("id")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("{id}_c{i}"));
                let user = c
                    .get("userName")
                    .or_else(|| c.get("user_name"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let content = c
                    .get("content")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let likes_c = c
                    .get("likeCount")
                    .or_else(|| c.get("like_count"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0);
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO competitor_comments (id, competitor_id, user_name, content, like_count, created_at)
                     VALUES (?1,?2,?3,?4,?5, datetime('now'))",
                    params![cid, id, user, content, likes_c],
                );
            }
        }
    }

    let trigger = body
        .trigger
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("手动刷新");
    let _ = conn.execute(
        "INSERT INTO refresh_logs (target_id, status, message, created_at)
         VALUES (?1, '成功', ?2, datetime('now', 'localtime'))",
        params![id, trigger],
    );

    conn.query_row(&format!("{COMP_SELECT} WHERE id = ?1"), params![id], map_comp)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn competitor_snapshots(
    db: State<'_, LibraryState>,
    competitor_id: String,
    days: Option<i64>,
) -> Result<Vec<SnapshotRow>, String> {
    let days = days.unwrap_or(30).clamp(1, 90);
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, competitor_id, likes, collects, comments, shares, engagement, captured_at,
                    COALESCE(source, 'live')
             FROM competitor_snapshots
             WHERE competitor_id = ?1 AND captured_at >= datetime('now', 'localtime', ?2)
             ORDER BY captured_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let window = format!("-{days} days");
    let rows = stmt
        .query_map(params![competitor_id.trim(), window], |row| {
            Ok(SnapshotRow {
                id: row.get(0)?,
                competitor_id: row.get(1)?,
                likes: row.get(2)?,
                collects: row.get(3)?,
                comments: row.get(4)?,
                shares: row.get(5)?,
                engagement: row.get(6)?,
                captured_at: row.get(7)?,
                source: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn scale_metric(v: i64, ratio: f64) -> i64 {
    ((v as f64) * ratio).round().max(0.0) as i64
}

/// 为监控笔记补齐近 N 天缺失的按日快照（线性爬升至锚点；source=backfill）。
#[tauri::command]
pub fn competitor_snapshots_backfill(
    db: State<'_, LibraryState>,
    days: Option<i64>,
    competitor_id: Option<String>,
) -> Result<Value, String> {
    let days = days.unwrap_or(30).clamp(2, 90);
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;

    let ids: Vec<String> = if let Some(cid) = competitor_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        vec![cid.to_string()]
    } else {
        let mut stmt = conn
            .prepare("SELECT id FROM competitors ORDER BY updated_at DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        out
    };

    let mut filled = 0i64;
    let mut touched = 0i64;

    for id in ids {
        let (likes0, collects0, comments0, shares0): (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT likes, collects, comments, shares FROM competitors WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap_or((0, 0, 0, 0));

        // 锚点：窗口内最早一条 live 快照；否则任意最早；再否则用当前笔记指标
        let anchor: (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT likes, collects, comments, shares FROM competitor_snapshots
                 WHERE competitor_id = ?1
                   AND captured_at >= datetime('now','localtime', ?2)
                   AND COALESCE(source,'live') = 'live'
                 ORDER BY captured_at ASC LIMIT 1",
                params![id, format!("-{} days", days)],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .or_else(|_| {
                conn.query_row(
                    "SELECT likes, collects, comments, shares FROM competitor_snapshots
                     WHERE competitor_id = ?1
                       AND captured_at >= datetime('now','localtime', ?2)
                     ORDER BY captured_at ASC LIMIT 1",
                    params![id, format!("-{} days", days)],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
            })
            .unwrap_or((likes0, collects0, comments0, shares0));

        if anchor.0 + anchor.1 + anchor.2 + anchor.3 <= 0 && likes0 + collects0 + comments0 + shares0 <= 0
        {
            continue;
        }
        let (a_likes, a_collects, a_comments, a_shares) = if anchor.0 + anchor.1 + anchor.2 + anchor.3 > 0
        {
            anchor
        } else {
            (likes0, collects0, comments0, shares0)
        };

        let mut existing_days = std::collections::HashSet::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT date(captured_at) FROM competitor_snapshots
                     WHERE competitor_id = ?1
                       AND captured_at >= datetime('now','localtime', ?2)",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![id, format!("-{} days", days)], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|e| e.to_string())?;
            for r in rows {
                if let Ok(d) = r {
                    if !d.is_empty() {
                        existing_days.insert(d);
                    }
                }
            }
        }

        let mut inserted_here = 0i64;
        for i in 0..days {
            let offset = days - 1 - i;
            let day: String = conn
                .query_row(
                    "SELECT date('now','localtime', ?1)",
                    params![format!("-{offset} days")],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if existing_days.contains(&day) {
                continue;
            }
            let t = if days <= 1 {
                1.0
            } else {
                i as f64 / (days - 1) as f64
            };
            // 线性爬升：最旧日约 55% 锚点 → 最近日 100% 锚点
            let ratio = 0.55 + 0.45 * t;
            let likes = scale_metric(a_likes, ratio);
            let collects = scale_metric(a_collects, ratio);
            let comments = scale_metric(a_comments, ratio);
            let shares = scale_metric(a_shares, ratio);
            let engagement = likes + collects + comments + shares;
            let captured = format!("{day} 12:00:00");
            conn.execute(
                "INSERT INTO competitor_snapshots
                 (competitor_id, likes, collects, comments, shares, engagement, captured_at, source)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,'backfill')",
                params![id, likes, collects, comments, shares, engagement, captured],
            )
            .map_err(|e| e.to_string())?;
            inserted_here += 1;
            filled += 1;
        }
        if inserted_here > 0 {
            touched += 1;
        }
    }

    Ok(serde_json::json!({
        "ok": true,
        "days": days,
        "competitorsTouched": touched,
        "snapshotsFilled": filled,
    }))
}

#[tauri::command]
pub fn competitor_set_chat(
    db: State<'_, LibraryState>,
    id: String,
    chat_json: String,
) -> Result<(), String> {
    let _: Value =
        serde_json::from_str(&chat_json).map_err(|e| format!("chatJson 无效: {e}"))?;
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    conn.execute(
        "UPDATE competitors SET chat_json=?2, updated_at=datetime('now') WHERE id=?1",
        params![id.trim(), chat_json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn competitor_set_analysis(
    db: State<'_, LibraryState>,
    id: String,
    analysis_json: String,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let (score, grade) = serde_json::from_str::<Value>(&analysis_json)
        .ok()
        .map(|v| {
            (
                v.get("score").and_then(|x| x.as_i64()).unwrap_or(0),
                v.get("grade")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .unwrap_or((0, String::new()));
    conn.execute(
        "UPDATE competitors SET analysis_json=?2, analysis_score=?3, analysis_grade=?4, updated_at=datetime('now') WHERE id=?1",
        params![id.trim(), analysis_json, score, grade],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn competitor_comments(
    db: State<'_, LibraryState>,
    competitor_id: String,
) -> Result<Vec<CommentRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, competitor_id, user_name, content, like_count, created_at
             FROM competitor_comments WHERE competitor_id = ?1 ORDER BY like_count DESC, created_at DESC LIMIT 100",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![competitor_id.trim()], |row| {
            Ok(CommentRow {
                id: row.get(0)?,
                competitor_id: row.get(1)?,
                user_name: row.get(2)?,
                content: row.get(3)?,
                like_count: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn competitor_refresh_logs(
    db: State<'_, LibraryState>,
    target_id: String,
) -> Result<Vec<RefreshLogRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, target_id, status, message, created_at FROM refresh_logs
             WHERE target_id = ?1 ORDER BY id DESC LIMIT 50",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![target_id.trim()], |row| {
            Ok(RefreshLogRow {
                id: row.get(0)?,
                target_id: row.get(1)?,
                status: row.get(2)?,
                message: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn competitor_get_settings(db: State<'_, LibraryState>) -> Result<Value, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let keywords: String = conn
        .query_row(
            "SELECT value FROM app_kv WHERE key = 'opp_keywords'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "[]".into());
    let interval: String = conn
        .query_row(
            "SELECT value FROM app_kv WHERE key = 'opp_scan_interval'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "60".into());
    Ok(serde_json::json!({
        "keywords": serde_json::from_str::<Value>(&keywords).unwrap_or(serde_json::json!([])),
        "scanIntervalMinutes": interval.parse::<i64>().unwrap_or(60),
    }))
}

#[tauri::command]
pub fn competitor_put_settings(
    db: State<'_, LibraryState>,
    keywords: Option<Vec<String>>,
    scan_interval_minutes: Option<i64>,
) -> Result<Value, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    if let Some(kw) = keywords {
        let v = serde_json::to_string(&kw).unwrap_or_else(|_| "[]".into());
        conn.execute(
            "INSERT INTO app_kv (key, value, updated_at) VALUES ('opp_keywords', ?1, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=datetime('now')",
            params![v],
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(mins) = scan_interval_minutes {
        let m = mins.max(30);
        conn.execute(
            "INSERT INTO app_kv (key, value, updated_at) VALUES ('opp_scan_interval', ?1, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=datetime('now')",
            params![m.to_string()],
        )
        .map_err(|e| e.to_string())?;
        // also update all competitor intervals for display parity
        conn.execute(
            "UPDATE competitors SET interval_minutes = ?1",
            params![m],
        )
        .map_err(|e| e.to_string())?;
    }
    drop(conn);
    competitor_get_settings(db)
}

#[tauri::command]
pub fn competitor_scan_opportunities(
    db: State<'_, LibraryState>,
    competitor_id: String,
) -> Result<Value, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let id = competitor_id.trim().to_string();
    let global_kw: String = conn
        .query_row(
            "SELECT value FROM app_kv WHERE key = 'opp_keywords'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "[]".into());
    let note_kw: String = conn
        .query_row(
            "SELECT keywords_json FROM competitors WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|_| "竞品不存在".to_string())?;
    let mut kws: Vec<String> = Vec::new();
    for raw in [global_kw, note_kw] {
        if let Ok(arr) = serde_json::from_str::<Vec<String>>(&raw) {
            for k in arr {
                let t = k.trim().to_string();
                if !t.is_empty() && !kws.contains(&t) {
                    kws.push(t);
                }
            }
        }
    }
    if kws.is_empty() {
        return Err("请先配置全局或本帖商机关键词".into());
    }

    let mut stmt = conn
        .prepare(
            "SELECT id, user_name, content FROM competitor_comments WHERE competitor_id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let comments: Vec<(String, String, String)> = stmt
        .query_map(params![id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let _ = conn.execute(
        "DELETE FROM opportunities WHERE competitor_id = ?1",
        params![id],
    );
    let mut created = 0i64;
    for (_cid, user, content) in comments {
        let lower = content.to_lowercase();
        for kw in &kws {
            if content.contains(kw) || lower.contains(&kw.to_lowercase()) {
                let oid = new_id("o");
                conn.execute(
                    "INSERT INTO opportunities (id, competitor_id, keyword, comment_text, author_nickname, status, created_at)
                     VALUES (?1,?2,?3,?4,?5,'open', datetime('now'))",
                    params![oid, id, kw, content, user],
                )
                .map_err(|e| e.to_string())?;
                created += 1;
                break;
            }
        }
    }
    conn.execute(
        "UPDATE competitors SET opportunity_count = ?2, updated_at=datetime('now') WHERE id = ?1",
        params![id, created],
    )
    .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "created": created }))
}

#[tauri::command]
pub fn competitor_list_opportunities(
    db: State<'_, LibraryState>,
    competitor_id: String,
) -> Result<Vec<OpportunityRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, competitor_id, keyword, comment_text, author_nickname, status, created_at
             FROM opportunities WHERE competitor_id = ?1 ORDER BY created_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![competitor_id.trim()], |row| {
            Ok(OpportunityRow {
                id: row.get(0)?,
                competitor_id: row.get(1)?,
                keyword: row.get(2)?,
                comment_text: row.get(3)?,
                author_nickname: row.get(4)?,
                status: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn competitor_delete_opportunity(
    db: State<'_, LibraryState>,
    id: String,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let competitor_id: String = conn
        .query_row(
            "SELECT competitor_id FROM opportunities WHERE id = ?1",
            params![id.trim()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    conn.execute("DELETE FROM opportunities WHERE id = ?1", params![id.trim()])
        .map_err(|e| e.to_string())?;
    if !competitor_id.is_empty() {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM opportunities WHERE competitor_id = ?1",
                params![competitor_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let _ = conn.execute(
            "UPDATE competitors SET opportunity_count = ?2 WHERE id = ?1",
            params![competitor_id, count],
        );
    }
    Ok(())
}

#[tauri::command]
pub fn competitor_set_keywords(
    db: State<'_, LibraryState>,
    id: String,
    keywords: Vec<String>,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let v = serde_json::to_string(&keywords).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "UPDATE competitors SET keywords_json=?2, updated_at=datetime('now') WHERE id=?1",
        params![id.trim(), v],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn map_wu(row: &rusqlite::Row<'_>) -> rusqlite::Result<WatchedUserRow> {
    Ok(WatchedUserRow {
        id: row.get(0)?,
        external_user_id: row.get(1)?,
        nickname: row.get(2)?,
        avatar_url: row.get(3)?,
        profile_url: row.get(4)?,
        note_count: row.get(5)?,
        last_synced_at: row.get(6)?,
        interval_minutes: row.get(7)?,
        next_run_at: row.get(8)?,
        status: row.get(9)?,
        last_error: row.get(10)?,
        latest_notes_json: row.get(11)?,
        unread_events: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

const WU_SELECT: &str = "SELECT id, external_user_id, nickname, avatar_url, profile_url, note_count,
        last_synced_at, interval_minutes, next_run_at, status, last_error, latest_notes_json,
        unread_events, created_at, updated_at FROM watched_users";

#[tauri::command]
pub fn watched_user_list(db: State<'_, LibraryState>) -> Result<Vec<WatchedUserRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let mut stmt = conn
        .prepare(&format!("{WU_SELECT} ORDER BY updated_at DESC"))
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], map_wu).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn watched_user_add(
    db: State<'_, LibraryState>,
    body: AddWatchedBody,
) -> Result<WatchedUserRow, String> {
    let url = body.profile_url.trim().to_string();
    if url.is_empty() {
        return Err("用户主页链接不能为空".into());
    }
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM watched_users", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if count >= MAX_WATCHED {
        return Err(format!("用户监控最多 {MAX_WATCHED} 个"));
    }
    if let Ok(existing) = conn.query_row(
        &format!("{WU_SELECT} WHERE profile_url = ?1"),
        params![url],
        map_wu,
    ) {
        return Ok(existing);
    }
    let id = new_id("u");
    conn.execute(
        "INSERT INTO watched_users (
            id, external_user_id, nickname, avatar_url, profile_url, note_count, last_synced_at,
            interval_minutes, next_run_at, status, last_error, latest_notes_json, unread_events, created_at, updated_at
         ) VALUES (?1,?2,?3,?4,?5,0,'',360,datetime('now','localtime','+360 minutes'),'active','','[]',0, datetime('now','localtime'), datetime('now','localtime'))",
        params![
            id,
            body.external_user_id.unwrap_or_default(),
            body.nickname.unwrap_or_default(),
            body.avatar_url.unwrap_or_default(),
            url
        ],
    )
    .map_err(|e| e.to_string())?;
    conn.query_row(&format!("{WU_SELECT} WHERE id = ?1"), params![id], map_wu)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn watched_user_delete(db: State<'_, LibraryState>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    conn.execute(
        "DELETE FROM watched_user_events WHERE watched_user_id = ?1",
        params![id.trim()],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM watched_users WHERE id = ?1", params![id.trim()])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn watched_user_refresh(
    db: State<'_, LibraryState>,
    body: RefreshWatchedBody,
) -> Result<WatchedUserRow, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let id = body.id.trim().to_string();
    let prev_notes: String = conn
        .query_row(
            "SELECT latest_notes_json FROM watched_users WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|_| "用户不存在".to_string())?;

    let notes_json = body
        .latest_notes_json
        .unwrap_or_else(|| prev_notes.clone());
    let note_count = body.note_count.unwrap_or(0);

    conn.execute(
        "UPDATE watched_users SET
            nickname=CASE WHEN length(trim(?2))>0 THEN ?2 ELSE nickname END,
            avatar_url=CASE WHEN length(trim(?3))>0 THEN ?3 ELSE avatar_url END,
            note_count=?4,
            latest_notes_json=?5,
            last_error=?6,
            last_synced_at=datetime('now', 'localtime'),
            next_run_at=datetime('now', 'localtime', '+360 minutes'),
            updated_at=datetime('now', 'localtime')
         WHERE id=?1",
        params![
            id,
            body.nickname.unwrap_or_default(),
            body.avatar_url.unwrap_or_default(),
            note_count,
            notes_json,
            body.last_error.unwrap_or_default()
        ],
    )
    .map_err(|e| e.to_string())?;

    if let Some(events) = body.new_events {
        for ev in events {
            let eid = new_id("e");
            conn.execute(
                "INSERT INTO watched_user_events (id, watched_user_id, event_type, title, note_url, note_id, is_read, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,0, datetime('now'))",
                params![
                    eid,
                    id,
                    ev.event_type.unwrap_or_else(|| "new_note".into()),
                    ev.title.unwrap_or_default(),
                    ev.note_url.unwrap_or_default(),
                    ev.note_id.unwrap_or_default()
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        let unread: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM watched_user_events WHERE watched_user_id = ?1 AND is_read = 0",
                params![id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let _ = conn.execute(
            "UPDATE watched_users SET unread_events = ?2 WHERE id = ?1",
            params![id, unread],
        );
    }

    conn.query_row(&format!("{WU_SELECT} WHERE id = ?1"), params![id], map_wu)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn watched_user_events(
    db: State<'_, LibraryState>,
    watched_user_id: String,
) -> Result<Vec<WatchedEventRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, watched_user_id, event_type, title, note_url, note_id, is_read, created_at
             FROM watched_user_events WHERE watched_user_id = ?1 ORDER BY created_at DESC LIMIT 100",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![watched_user_id.trim()], |row| {
            Ok(WatchedEventRow {
                id: row.get(0)?,
                watched_user_id: row.get(1)?,
                event_type: row.get(2)?,
                title: row.get(3)?,
                note_url: row.get(4)?,
                note_id: row.get(5)?,
                is_read: row.get::<_, i64>(6)? != 0,
                created_at: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn watched_user_mark_read(
    db: State<'_, LibraryState>,
    watched_user_id: String,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_competitor_schema(&conn)?;
    conn.execute(
        "UPDATE watched_user_events SET is_read = 1 WHERE watched_user_id = ?1",
        params![watched_user_id.trim()],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE watched_users SET unread_events = 0 WHERE id = ?1",
        params![watched_user_id.trim()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
