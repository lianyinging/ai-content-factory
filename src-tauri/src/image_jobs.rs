//! Async image generation / edit jobs persisted in SQLite.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};

use crate::assets::{assets_root, fetch_asset_row, insert_asset_row, MediaAsset};
use crate::library::LibraryState;
use crate::node_jobs::run_ai_action;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageJob {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub prompt: String,
    pub model: String,
    pub size: String,
    pub quality: String,
    pub base_image_path: String,
    pub draft_id: String,
    pub folder_id: String,
    pub platform: String,
    pub asset_id: String,
    pub abs_path: String,
    pub error: String,
    pub created_at: String,
    pub started_at: String,
    pub finished_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageJobStartBody {
    pub kind: String,
    pub prompt: String,
    pub size: Option<String>,
    pub quality: Option<String>,
    pub base_image_path: Option<String>,
    pub draft_id: Option<String>,
    pub folder_id: Option<String>,
    pub platform: Option<String>,
    pub llm: Value,
    pub timeout_secs: Option<u64>,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn new_job_id() -> String {
    format!("ij_{}", now_ms())
}

pub fn ensure_image_jobs_table(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS image_jobs (
            id TEXT PRIMARY KEY NOT NULL,
            kind TEXT NOT NULL DEFAULT 'generate',
            status TEXT NOT NULL DEFAULT 'queued',
            prompt TEXT NOT NULL DEFAULT '',
            model TEXT NOT NULL DEFAULT '',
            size TEXT NOT NULL DEFAULT '',
            quality TEXT NOT NULL DEFAULT '',
            base_image_path TEXT NOT NULL DEFAULT '',
            draft_id TEXT NOT NULL DEFAULT '',
            folder_id TEXT NOT NULL DEFAULT '',
            platform TEXT NOT NULL DEFAULT '',
            asset_id TEXT NOT NULL DEFAULT '',
            abs_path TEXT NOT NULL DEFAULT '',
            error TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            started_at TEXT NOT NULL DEFAULT '',
            finished_at TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_image_jobs_status_created
            ON image_jobs(status, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_image_jobs_draft
            ON image_jobs(draft_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_image_jobs_folder
            ON image_jobs(folder_id, created_at DESC);
        "#,
    )
    .map_err(|e| format!("image_jobs migrate: {e}"))?;
    Ok(())
}

pub fn mark_interrupted_jobs(conn: &rusqlite::Connection) -> Result<u64, String> {
    let n = conn
        .execute(
            "UPDATE image_jobs SET status = 'error', error = '应用中断', finished_at = datetime('now')
             WHERE status IN ('queued', 'running')",
            [],
        )
        .map_err(|e| e.to_string())?;
    Ok(n as u64)
}

fn map_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImageJob> {
    Ok(ImageJob {
        id: row.get(0)?,
        kind: row.get(1)?,
        status: row.get(2)?,
        prompt: row.get(3)?,
        model: row.get(4)?,
        size: row.get(5)?,
        quality: row.get(6)?,
        base_image_path: row.get(7)?,
        draft_id: row.get(8)?,
        folder_id: row.get(9)?,
        platform: row.get(10)?,
        asset_id: row.get(11)?,
        abs_path: row.get(12)?,
        error: row.get(13)?,
        created_at: row.get(14)?,
        started_at: row.get(15)?,
        finished_at: row.get(16)?,
    })
}

const JOB_SELECT: &str = "SELECT id, kind, status, prompt, model, size, quality, base_image_path,
    draft_id, folder_id, platform, asset_id, abs_path, error, created_at, started_at, finished_at
    FROM image_jobs";

fn fetch_job(conn: &rusqlite::Connection, id: &str) -> Result<ImageJob, String> {
    conn.query_row(
        &format!("{JOB_SELECT} WHERE id = ?1"),
        params![id],
        map_job,
    )
    .map_err(|e| format!("任务不存在: {e}"))
}

fn ext_for(mime: &str) -> &str {
    let m = mime.to_ascii_lowercase();
    if m.contains("jpeg") || m.contains("jpg") {
        "jpg"
    } else if m.contains("webp") {
        "webp"
    } else if m.contains("gif") {
        "gif"
    } else {
        "png"
    }
}

fn ingest_local_file(
    app: &AppHandle,
    db: &LibraryState,
    path: &str,
    source: &str,
    prompt: &str,
    model: &str,
    draft_id: &str,
    platform: &str,
    folder_id: &str,
) -> Result<MediaAsset, String> {
    let src = PathBuf::from(path);
    if !src.is_file() {
        return Err(format!("结果文件不存在: {path}"));
    }
    let bytes = fs::read(&src).map_err(|e| format!("读取结果失败: {e}"))?;
    if bytes.is_empty() {
        return Err("结果文件为空".into());
    }
    let root = assets_root(app)?;
    let mime = match src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/png",
    };
    let id = format!("as_{}", now_ms());
    let file_name = format!("{id}.{}", ext_for(mime));
    let rel_path = format!("images/{file_name}");
    let abs_path = root.join(&rel_path);
    fs::create_dir_all(abs_path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&abs_path, &bytes).map_err(|e| format!("写入素材失败: {e}"))?;

    let asset = MediaAsset {
        id: id.clone(),
        kind: "image".into(),
        file_name,
        rel_path,
        abs_path: abs_path.to_string_lossy().to_string(),
        mime: mime.into(),
        size_bytes: bytes.len() as i64,
        source: source.into(),
        prompt: prompt.into(),
        model: model.into(),
        draft_id: draft_id.into(),
        platform: platform.trim().to_ascii_lowercase(),
        folder_id: folder_id.into(),
        sort_order: 0,
        created_at: String::new(),
    };

    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    insert_asset_row(&conn, &asset)?;
    fetch_asset_row(&conn, &id)
}

fn run_job_worker(app: AppHandle, db: &LibraryState, job_id: String, body: ImageJobStartBody) {
    if let Ok(conn) = db.conn.lock() {
        let _ = conn.execute(
            "UPDATE image_jobs SET status = 'running', started_at = datetime('now') WHERE id = ?1",
            params![job_id],
        );
    }

    let kind = body.kind.trim().to_ascii_lowercase();
    let action = if kind == "edit" {
        "image-edit"
    } else {
        "image-generate"
    };
    let source = if kind == "edit" {
        "ai-edit"
    } else {
        "ai-generate"
    };
    let size = body
        .size
        .as_deref()
        .unwrap_or("1024x1024")
        .trim()
        .to_string();
    let quality = body
        .quality
        .as_deref()
        .unwrap_or("medium")
        .trim()
        .to_string();
    let prompt = body.prompt.trim().to_string();
    let timeout = body.timeout_secs.unwrap_or(180).max(60);
    let model = body
        .llm
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut payload = json!({
        "llm": body.llm,
        "prompt": prompt,
        "size": size,
        "quality": quality,
    });
    if kind == "edit" {
        payload["imageUrl"] = json!(body.base_image_path.clone().unwrap_or_default());
    }

    let result = run_ai_action(&app, action, &payload, timeout);
    let draft_id = body.draft_id.unwrap_or_default();
    let folder_id = body.folder_id.unwrap_or_default();
    let platform = body.platform.unwrap_or_default();

    if !result.ok {
        let err = result.error.unwrap_or_else(|| "图片任务失败".into());
        if let Ok(conn) = db.conn.lock() {
            let _ = conn.execute(
                "UPDATE image_jobs SET status = 'error', error = ?2, finished_at = datetime('now') WHERE id = ?1",
                params![job_id, err],
            );
        }
        return;
    }

    let path = result
        .result
        .as_ref()
        .and_then(|v| {
            v.get("path")
                .or_else(|| v.get("url"))
                .and_then(|x| x.as_str())
        })
        .unwrap_or("")
        .to_string();
    let out_model = result
        .result
        .as_ref()
        .and_then(|v| v.get("model"))
        .and_then(|x| x.as_str())
        .unwrap_or(&model)
        .to_string();

    if path.is_empty() {
        if let Ok(conn) = db.conn.lock() {
            let _ = conn.execute(
                "UPDATE image_jobs SET status = 'error', error = '未返回图片路径', finished_at = datetime('now') WHERE id = ?1",
                params![job_id],
            );
        }
        return;
    }

    // Remote http URL: download via ingest path reader later — Node already persists b64 to local path
    let local_path = if path.starts_with("http://") || path.starts_with("https://") {
        // download to temp then ingest
        match download_to_temp(&path) {
            Ok(p) => p,
            Err(e) => {
                if let Ok(conn) = db.conn.lock() {
                    let _ = conn.execute(
                        "UPDATE image_jobs SET status = 'error', error = ?2, finished_at = datetime('now') WHERE id = ?1",
                        params![job_id, e],
                    );
                }
                return;
            }
        }
    } else {
        path.clone()
    };

    match ingest_local_file(
        &app,
        db,
        &local_path,
        source,
        &prompt,
        &out_model,
        &draft_id,
        &platform,
        &folder_id,
    ) {
        Ok(asset) => {
            if let Ok(conn) = db.conn.lock() {
                let _ = conn.execute(
                    "UPDATE image_jobs SET status = 'done', asset_id = ?2, abs_path = ?3, model = ?4, error = '', finished_at = datetime('now') WHERE id = ?1",
                    params![job_id, asset.id, asset.abs_path, out_model],
                );
            }
            if local_path.contains("ai-content-factory-images") {
                let _ = fs::remove_file(&local_path);
            }
        }
        Err(e) => {
            if let Ok(conn) = db.conn.lock() {
                let _ = conn.execute(
                    "UPDATE image_jobs SET status = 'error', error = ?2, finished_at = datetime('now') WHERE id = ?1",
                    params![job_id, e],
                );
            }
        }
    }
}

fn download_to_temp(url: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout_read(std::time::Duration::from_secs(120))
        .build();
    let res = agent
        .get(url)
        .call()
        .map_err(|e| format!("下载图片失败: {e}"))?;
    let mut bytes = Vec::new();
    res.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("读取图片失败: {e}"))?;
    let dir = std::env::temp_dir().join("ai-content-factory-images");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("dl_{}.png", now_ms()));
    fs::write(&file, &bytes).map_err(|e| e.to_string())?;
    Ok(file.to_string_lossy().to_string())
}

#[tauri::command]
pub fn image_job_start(
    app: AppHandle,
    db: State<'_, LibraryState>,
    body: ImageJobStartBody,
) -> Result<ImageJob, String> {
    let kind = body.kind.trim().to_ascii_lowercase();
    if kind != "generate" && kind != "edit" {
        return Err("kind 须为 generate 或 edit".into());
    }
    let prompt = body.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err("请填写提示词".into());
    }
    if kind == "edit" {
        let base = body.base_image_path.as_deref().unwrap_or("").trim();
        if base.is_empty() {
            return Err("图生图需要底图".into());
        }
    }
    let model = body
        .llm
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if body
        .llm
        .get("apiKey")
        .or_else(|| body.llm.get("api_key"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        return Err("请先配置图片大模型 API Key".into());
    }

    let id = new_job_id();
    let size = body.size.clone().unwrap_or_else(|| "1024x1024".into());
    let quality = body.quality.clone().unwrap_or_else(|| "medium".into());
    let base_image_path = body.base_image_path.clone().unwrap_or_default();
    let draft_id = body.draft_id.clone().unwrap_or_default();
    let folder_id = body.folder_id.clone().unwrap_or_default();
    let platform = body
        .platform
        .clone()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO image_jobs
             (id, kind, status, prompt, model, size, quality, base_image_path, draft_id, folder_id, platform, created_at)
             VALUES (?1,?2,'queued',?3,?4,?5,?6,?7,?8,?9,?10, datetime('now'))",
            params![
                id,
                kind,
                prompt,
                model,
                size,
                quality,
                base_image_path,
                draft_id,
                folder_id,
                platform,
            ],
        )
        .map_err(|e| format!("创建任务失败: {e}"))?;
    }

    let job = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        fetch_job(&conn, &id)?
    };

    let app2 = app.clone();
    let job_id = id.clone();
    let body2 = body;
    std::thread::spawn(move || {
        let Some(state) = app2.try_state::<LibraryState>() else {
            return;
        };
        run_job_worker(app2.clone(), state.inner(), job_id, body2);
    });

    Ok(job)
}

#[tauri::command]
pub fn image_job_get(db: State<'_, LibraryState>, id: String) -> Result<Option<ImageJob>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        &format!("{JOB_SELECT} WHERE id = ?1"),
        params![id.trim()],
        map_job,
    )
    .optional()
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn image_job_delete(db: State<'_, LibraryState>, id: String) -> Result<Value, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let n = conn
        .execute("DELETE FROM image_jobs WHERE id = ?1", params![id.trim()])
        .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "deleted": n }))
}

#[tauri::command]
pub fn image_job_list(
    db: State<'_, LibraryState>,
    draft_id: Option<String>,
    folder_id: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<ImageJob>, String> {
    let lim = limit.unwrap_or(100).clamp(1, 500);
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    let draft = draft_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let folder_filter = folder_id.is_some();
    let folder = folder_id.unwrap_or_default();
    let st = status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let mut sql = format!("{JOB_SELECT} WHERE 1=1");
    if draft.is_some() {
        sql.push_str(" AND draft_id = ?");
    }
    if folder_filter {
        sql.push_str(" AND folder_id = ?");
    }
    if st.is_some() {
        sql.push_str(" AND status = ?");
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT ?");

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(d) = draft {
        vals.push(Box::new(d.to_string()));
    }
    if folder_filter {
        vals.push(Box::new(folder));
    }
    if let Some(s) = st {
        vals.push(Box::new(s.to_string()));
    }
    vals.push(Box::new(lim));
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(params_ref.as_slice(), map_job)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}
