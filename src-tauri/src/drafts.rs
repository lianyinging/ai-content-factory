use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::library::{ensure_library_media_columns, LibraryState};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftRow {
    pub id: String,
    pub platform: String,
    pub title: String,
    pub body: String,
    pub tags_json: String,
    pub image_urls_json: String,
    pub source_item_id: String,
    pub status: String,
    pub updated_at: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertDraftBody {
    pub id: Option<String>,
    pub platform: String,
    pub title: Option<String>,
    pub body: Option<String>,
    pub tags_json: Option<String>,
    pub image_urls_json: Option<String>,
    pub source_item_id: Option<String>,
    pub status: Option<String>,
}

fn new_id() -> String {
    format!(
        "d_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    )
}

fn map_draft(row: &rusqlite::Row<'_>) -> rusqlite::Result<DraftRow> {
    Ok(DraftRow {
        id: row.get(0)?,
        platform: row.get(1)?,
        title: row.get(2)?,
        body: row.get(3)?,
        tags_json: row.get(4)?,
        image_urls_json: row.get(5)?,
        source_item_id: row.get(6)?,
        status: row.get(7)?,
        updated_at: row.get(8)?,
        created_at: row.get(9)?,
    })
}

#[tauri::command]
pub fn draft_list(
    db: State<'_, LibraryState>,
    platform: String,
) -> Result<Vec<DraftRow>, String> {
    let plat = platform.trim().to_lowercase();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, platform, title, body, tags_json, image_urls_json, source_item_id, status, updated_at, created_at
             FROM drafts WHERE platform = ?1 ORDER BY updated_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![plat], map_draft)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn draft_get(db: State<'_, LibraryState>, id: String) -> Result<Option<DraftRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT id, platform, title, body, tags_json, image_urls_json, source_item_id, status, updated_at, created_at
         FROM drafts WHERE id = ?1",
        params![id.trim()],
        map_draft,
    )
    .optional()
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn draft_upsert(
    db: State<'_, LibraryState>,
    body: UpsertDraftBody,
) -> Result<DraftRow, String> {
    let plat = body.platform.trim().to_lowercase();
    if plat != "xhs" && plat != "douyin" {
        return Err(format!("unsupported platform: {}", body.platform));
    }
    let id = body
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(new_id);
    let title = body.title.unwrap_or_default();
    let draft_body = body.body.unwrap_or_default();
    let tags = body.tags_json.unwrap_or_else(|| "[]".into());
    let images = body.image_urls_json.unwrap_or_else(|| "[]".into());
    let source = body.source_item_id.unwrap_or_default();
    let status = body.status.unwrap_or_else(|| "editing".into());

    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO drafts (id, platform, title, body, tags_json, image_urls_json, source_item_id, status, updated_at, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8, datetime('now'), datetime('now'))
         ON CONFLICT(id) DO UPDATE SET
            title=excluded.title,
            body=excluded.body,
            tags_json=excluded.tags_json,
            image_urls_json=excluded.image_urls_json,
            source_item_id=excluded.source_item_id,
            status=excluded.status,
            updated_at=datetime('now')",
        params![id, plat, title, draft_body, tags, images, source, status],
    )
    .map_err(|e| e.to_string())?;

    conn.query_row(
        "SELECT id, platform, title, body, tags_json, image_urls_json, source_item_id, status, updated_at, created_at
         FROM drafts WHERE id = ?1",
        params![id],
        map_draft,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn draft_delete(db: State<'_, LibraryState>, id: String) -> Result<Value, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let n = conn
        .execute("DELETE FROM drafts WHERE id = ?1", params![id.trim()])
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "deleted": n }))
}

#[tauri::command]
pub fn library_to_drafts(
    db: State<'_, LibraryState>,
    platform: String,
    item_ids: Vec<String>,
) -> Result<Value, String> {
    let plat = platform.trim().to_lowercase();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    ensure_library_media_columns(&conn)?;
    let mut created = 0i64;
    for item_id in item_ids {
        let row: Option<(String, String, String, String, String, String)> = conn
            .query_row(
                "SELECT title, desc_text, cover_url, COALESCE(cover_local_path,''),
                        COALESCE(images_local_json,'[]'), item_id
                 FROM library_items WHERE platform=?1 AND item_id=?2",
                params![plat, item_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((title, body, cover_url, cover_local, images_local_json, src)) = row else {
            continue;
        };
        let id = new_id();
        let cover = if !cover_local.trim().is_empty() {
            cover_local
        } else {
            cover_url
        };
        let images = {
            let local_arr: Vec<String> =
                serde_json::from_str(&images_local_json).unwrap_or_default();
            if !local_arr.is_empty() {
                serde_json::to_string(&local_arr).unwrap_or_else(|_| "[]".into())
            } else if cover.trim().is_empty() {
                "[]".to_string()
            } else {
                serde_json::to_string(&vec![cover]).unwrap_or_else(|_| "[]".into())
            }
        };
        conn.execute(
            "INSERT INTO drafts (id, platform, title, body, tags_json, image_urls_json, source_item_id, status, updated_at, created_at)
             VALUES (?1,?2,?3,?4,'[]',?5,?6,'editing', datetime('now'), datetime('now'))",
            params![id, plat, title, body, images, src],
        )
        .map_err(|e| e.to_string())?;
        created += 1;
    }
    Ok(serde_json::json!({ "ok": true, "created": created }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishLogRow {
    pub id: String,
    pub platform: String,
    pub draft_id: String,
    pub title: String,
    pub action: String,
    pub ok: bool,
    pub message: String,
    pub steps_json: String,
    pub profile_id: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddPublishLogBody {
    pub platform: String,
    pub draft_id: Option<String>,
    pub title: Option<String>,
    pub action: String,
    pub ok: bool,
    pub message: Option<String>,
    pub steps_json: Option<String>,
    pub profile_id: Option<String>,
}

fn map_publish_log(row: &rusqlite::Row<'_>) -> rusqlite::Result<PublishLogRow> {
    let ok_i: i64 = row.get(5)?;
    Ok(PublishLogRow {
        id: row.get(0)?,
        platform: row.get(1)?,
        draft_id: row.get(2)?,
        title: row.get(3)?,
        action: row.get(4)?,
        ok: ok_i != 0,
        message: row.get(6)?,
        steps_json: row.get(7)?,
        profile_id: row.get(8)?,
        created_at: row.get(9)?,
    })
}

#[tauri::command]
pub fn publish_log_list(
    db: State<'_, LibraryState>,
    platform: String,
    limit: Option<i64>,
) -> Result<Vec<PublishLogRow>, String> {
    let plat = platform.trim().to_lowercase();
    let lim = limit.unwrap_or(100).clamp(1, 500);
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, platform, draft_id, title, action, ok, message, steps_json, profile_id, created_at
             FROM publish_logs WHERE platform = ?1 ORDER BY created_at DESC LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![plat, lim], map_publish_log)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn publish_log_add(
    db: State<'_, LibraryState>,
    body: AddPublishLogBody,
) -> Result<PublishLogRow, String> {
    let plat = body.platform.trim().to_lowercase();
    if plat != "xhs" && plat != "douyin" {
        return Err(format!("unsupported platform: {}", body.platform));
    }
    let id = format!(
        "pl_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let draft_id = body.draft_id.unwrap_or_default();
    let title = body.title.unwrap_or_default();
    let action = body.action.trim().to_string();
    let message = body.message.unwrap_or_default();
    let steps = body.steps_json.unwrap_or_else(|| "[]".into());
    let profile_id = body.profile_id.unwrap_or_default();
    let ok = if body.ok { 1i64 } else { 0i64 };

    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO publish_logs (id, platform, draft_id, title, action, ok, message, steps_json, profile_id, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9, datetime('now'))",
        params![id, plat, draft_id, title, action, ok, message, steps, profile_id],
    )
    .map_err(|e| e.to_string())?;

    conn.query_row(
        "SELECT id, platform, draft_id, title, action, ok, message, steps_json, profile_id, created_at
         FROM publish_logs WHERE id = ?1",
        params![id],
        map_publish_log,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn publish_log_clear(
    db: State<'_, LibraryState>,
    platform: String,
) -> Result<Value, String> {
    let plat = platform.trim().to_lowercase();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let n = conn
        .execute("DELETE FROM publish_logs WHERE platform = ?1", params![plat])
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "deleted": n }))
}
