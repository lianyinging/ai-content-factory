use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, State};

use crate::competitors::ensure_competitor_schema;
use crate::library::LibraryState;
use crate::media_cache::cache_note_media;

/// Download Top5 covers into `note-media/industry-insight/` and inject `coverLocalPath`.
fn enrich_posts_with_local_covers(app: &AppHandle, platform: &str, posts_json: &str) -> String {
    let Ok(mut arr) = serde_json::from_str::<Vec<Value>>(posts_json) else {
        return posts_json.to_string();
    };
    for (i, post) in arr.iter_mut().enumerate() {
        let Some(obj) = post.as_object_mut() else {
            continue;
        };
        let cover_url = obj
            .get("coverUrl")
            .or_else(|| obj.get("cover_url"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if cover_url.is_empty() {
            continue;
        }
        let id = obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let key = if !id.is_empty() {
            format!("{platform}_{id}")
        } else {
            format!("{platform}_{i}")
        };
        let cached = cache_note_media(app, "industry-insight", &key, &cover_url, &[]);
        if !cached.cover_local.is_empty() {
            obj.insert("coverLocalPath".into(), json!(cached.cover_local));
        }
    }
    serde_json::to_string(&arr).unwrap_or_else(|_| posts_json.to_string())
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

fn clamp_interval_minutes(v: i64) -> i64 {
    v.clamp(15, 1440)
}

fn ensure_dashboard_schema(conn: &Connection) -> Result<(), String> {
    ensure_competitor_schema(conn)?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS dashboard_insight_config (
            platform TEXT PRIMARY KEY NOT NULL,
            remark TEXT NOT NULL DEFAULT '',
            industry_keyword TEXT NOT NULL DEFAULT '',
            interval_minutes INTEGER NOT NULL DEFAULT 30,
            updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
        );
        CREATE TABLE IF NOT EXISTS industry_insight_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            platform TEXT NOT NULL DEFAULT 'xhs',
            account TEXT NOT NULL DEFAULT '',
            keyword TEXT NOT NULL DEFAULT '',
            posts_json TEXT NOT NULL DEFAULT '[]',
            analysis_json TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT '',
            error_message TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
        );
        "#,
    )
    .map_err(|e| format!("dashboard schema: {e}"))?;
    if !has_column(conn, "dashboard_insight_config", "interval_minutes") {
        conn.execute(
            "ALTER TABLE dashboard_insight_config ADD COLUMN interval_minutes INTEGER NOT NULL DEFAULT 30",
            [],
        )
        .map_err(|e| format!("alter interval_minutes: {e}"))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardInsightConfig {
    pub platform: String,
    pub remark: String,
    pub industry_keyword: String,
    /// 自动刷新间隔（分钟），15～1440
    pub interval_minutes: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndustryInsightRun {
    pub id: i64,
    pub platform: String,
    pub account: String,
    pub keyword: String,
    pub posts_json: String,
    pub analysis_json: String,
    pub status: String,
    pub error_message: String,
    pub created_at: String,
}

#[tauri::command]
pub fn dashboard_insight_config_get(
    db: State<'_, LibraryState>,
    platform: String,
) -> Result<DashboardInsightConfig, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_dashboard_schema(&conn)?;
    let p = platform.trim().to_string();
    if p.is_empty() {
        return Err("platform required".into());
    }
    let row = conn
        .query_row(
            "SELECT platform, remark, industry_keyword, COALESCE(interval_minutes, 30), updated_at
             FROM dashboard_insight_config WHERE platform = ?1",
            params![p],
            |r| {
                Ok(DashboardInsightConfig {
                    platform: r.get(0)?,
                    remark: r.get(1)?,
                    industry_keyword: r.get(2)?,
                    interval_minutes: clamp_interval_minutes(r.get::<_, i64>(3).unwrap_or(30)),
                    updated_at: r.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(row.unwrap_or(DashboardInsightConfig {
        platform: p,
        remark: String::new(),
        industry_keyword: String::new(),
        interval_minutes: 30,
        updated_at: String::new(),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardInsightConfigBody {
    pub platform: String,
    pub remark: Option<String>,
    pub industry_keyword: Option<String>,
    pub interval_minutes: Option<i64>,
}

#[tauri::command]
pub fn dashboard_insight_config_upsert(
    db: State<'_, LibraryState>,
    body: DashboardInsightConfigBody,
) -> Result<DashboardInsightConfig, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_dashboard_schema(&conn)?;
    let p = body.platform.trim().to_string();
    if p.is_empty() {
        return Err("platform required".into());
    }
    let remark = body.remark.unwrap_or_default();
    let keyword = body.industry_keyword.unwrap_or_default();
    let interval = clamp_interval_minutes(body.interval_minutes.unwrap_or(30));
    conn.execute(
        "INSERT INTO dashboard_insight_config (platform, remark, industry_keyword, interval_minutes, updated_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now','localtime'))
         ON CONFLICT(platform) DO UPDATE SET
           remark=excluded.remark,
           industry_keyword=excluded.industry_keyword,
           interval_minutes=excluded.interval_minutes,
           updated_at=datetime('now','localtime')",
        params![p, remark, keyword, interval],
    )
    .map_err(|e| e.to_string())?;
    dashboard_insight_config_get_inner(&conn, &p)
}

fn dashboard_insight_config_get_inner(
    conn: &Connection,
    platform: &str,
) -> Result<DashboardInsightConfig, String> {
    conn.query_row(
        "SELECT platform, remark, industry_keyword, COALESCE(interval_minutes, 30), updated_at
         FROM dashboard_insight_config WHERE platform = ?1",
        params![platform],
        |r| {
            Ok(DashboardInsightConfig {
                platform: r.get(0)?,
                remark: r.get(1)?,
                industry_keyword: r.get(2)?,
                interval_minutes: clamp_interval_minutes(r.get::<_, i64>(3).unwrap_or(30)),
                updated_at: r.get(4)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndustryInsightRunBody {
    pub platform: Option<String>,
    pub account: Option<String>,
    pub keyword: Option<String>,
    pub posts_json: Option<String>,
    pub analysis_json: Option<String>,
    pub status: Option<String>,
    pub error_message: Option<String>,
}

#[tauri::command]
pub fn industry_insight_run_add(
    app: AppHandle,
    db: State<'_, LibraryState>,
    body: IndustryInsightRunBody,
) -> Result<IndustryInsightRun, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_dashboard_schema(&conn)?;
    let platform = body
        .platform
        .unwrap_or_else(|| "xhs".into())
        .trim()
        .to_string();
    let account = body.account.unwrap_or_default();
    let keyword = body.keyword.unwrap_or_default();
    let raw_posts = body.posts_json.unwrap_or_else(|| "[]".into());
    // 封面图落盘到 note-media/industry-insight/，并把本地路径写回 posts_json
    let posts_json = enrich_posts_with_local_covers(&app, &platform, &raw_posts);
    let analysis_json = body.analysis_json.unwrap_or_default();
    let status = body.status.unwrap_or_else(|| "ok".into());
    let error_message = body.error_message.unwrap_or_default();
    conn.execute(
        "INSERT INTO industry_insight_runs
         (platform, account, keyword, posts_json, analysis_json, status, error_message, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now','localtime'))",
        params![
            platform,
            account,
            keyword,
            posts_json,
            analysis_json,
            status,
            error_message
        ],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    // keep last 30 runs per platform
    let _ = conn.execute(
        "DELETE FROM industry_insight_runs WHERE platform = ?1 AND id NOT IN (
           SELECT id FROM industry_insight_runs WHERE platform = ?1 ORDER BY id DESC LIMIT 30
         )",
        params![platform],
    );
    conn.query_row(
        "SELECT id, platform, account, keyword, posts_json, analysis_json, status, error_message, created_at
         FROM industry_insight_runs WHERE id = ?1",
        params![id],
        |r| {
            Ok(IndustryInsightRun {
                id: r.get(0)?,
                platform: r.get(1)?,
                account: r.get(2)?,
                keyword: r.get(3)?,
                posts_json: r.get(4)?,
                analysis_json: r.get(5)?,
                status: r.get(6)?,
                error_message: r.get(7)?,
                created_at: r.get(8)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn industry_insight_run_latest(
    db: State<'_, LibraryState>,
    platform: String,
) -> Result<Option<IndustryInsightRun>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_dashboard_schema(&conn)?;
    let p = platform.trim().to_string();
    conn.query_row(
        "SELECT id, platform, account, keyword, posts_json, analysis_json, status, error_message, created_at
         FROM industry_insight_runs WHERE platform = ?1 ORDER BY id DESC LIMIT 1",
        params![p],
        |r| {
            Ok(IndustryInsightRun {
                id: r.get(0)?,
                platform: r.get(1)?,
                account: r.get(2)?,
                keyword: r.get(3)?,
                posts_json: r.get(4)?,
                analysis_json: r.get(5)?,
                status: r.get(6)?,
                error_message: r.get(7)?,
                created_at: r.get(8)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}
