//! Python knowledge sidecar (Chroma + local BGE).

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbJobResult {
    pub ok: bool,
    pub exit_code: i32,
    pub result: Option<Value>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
}

fn knowledge_dir(app: &AppHandle) -> PathBuf {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/knowledge"),
        app.path()
            .resource_dir()
            .map(|p| p.join("resources/knowledge"))
            .unwrap_or_default(),
        app.path()
            .resource_dir()
            .map(|p| p.join("knowledge"))
            .unwrap_or_default(),
    ];
    for c in candidates {
        if c.join("run.py").is_file() {
            return c.canonicalize().unwrap_or(c);
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/knowledge")
}

fn model_dir(app: &AppHandle) -> PathBuf {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/model/BAAI/bge-small-zh-v1.5"),
        app.path()
            .resource_dir()
            .map(|p| p.join("resources/model/BAAI/bge-small-zh-v1.5"))
            .unwrap_or_default(),
        app.path()
            .resource_dir()
            .map(|p| p.join("model/BAAI/bge-small-zh-v1.5"))
            .unwrap_or_default(),
    ];
    for c in &candidates {
        if c.is_dir() {
            return c.canonicalize().unwrap_or_else(|_| c.clone());
        }
    }
    candidates[0].clone()
}

fn workspace_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?
        .join("knowledge");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir knowledge: {e}"))?;
    Ok(dir)
}

fn resolve_python(app: &AppHandle) -> Result<String, String> {
    if let Ok(p) = std::env::var("AI_FACTORY_PYTHON") {
        let t = p.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }

    // Prefer dedicated venv under resources/knowledge/.venv (avoids global NumPy 2 vs Chroma clash).
    let kb = knowledge_dir(app);
    let venv_candidates = [
        kb.join(".venv/bin/python"),
        kb.join(".venv/bin/python3"),
        kb.join(".venv/Scripts/python.exe"),
    ];
    for p in &venv_candidates {
        if p.is_file() {
            return Ok(p.to_string_lossy().to_string());
        }
    }

    for cand in ["python3", "python"] {
        let status = Command::new(cand)
            .arg("-c")
            .arg("import sys; print(sys.executable)")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        if let Ok(out) = status {
            if out.status.success() {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(path);
                }
            }
        }
    }
    Err(
        "未找到 python3。请在 resources/knowledge 执行: python3 -m venv .venv && .venv/bin/pip install -r requirements.txt"
            .into(),
    )
}

fn run_kb(app: &AppHandle, mut payload: Value, timeout_secs: u64) -> KbJobResult {
    let dir = knowledge_dir(app);
    let entry = dir.join("run.py");
    if !entry.is_file() {
        return KbJobResult {
            ok: false,
            exit_code: -1,
            result: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(format!("missing {}", entry.display())),
        };
    }
    let python = match resolve_python(app) {
        Ok(p) => p,
        Err(e) => {
            return KbJobResult {
                ok: false,
                exit_code: -1,
                result: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(e),
            };
        }
    };
    let workspace = match workspace_dir(app) {
        Ok(p) => p,
        Err(e) => {
            return KbJobResult {
                ok: false,
                exit_code: -1,
                result: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(e),
            };
        }
    };
    if let Some(obj) = payload.as_object_mut() {
        obj.entry("workspace".to_string())
            .or_insert_with(|| json!(workspace.to_string_lossy()));
    }

    let model = model_dir(app);
    let stdin = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => {
            return KbJobResult {
                ok: false,
                exit_code: -1,
                result: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(e.to_string()),
            };
        }
    };

    let mut cmd = Command::new(&python);
    cmd.arg(&entry)
        .current_dir(&dir)
        .env("EMBEDDING_MODEL_PATH", &model)
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return KbJobResult {
                ok: false,
                exit_code: -1,
                result: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(format!("spawn python: {e}")),
            };
        }
    };

    if let Some(mut sin) = child.stdin.take() {
        let _ = sin.write_all(stdin.as_bytes());
    }

    let timeout = Duration::from_secs(timeout_secs.max(5));
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return KbJobResult {
                        ok: false,
                        exit_code: -1,
                        result: None,
                        stdout: String::new(),
                        stderr: String::new(),
                        error: Some("knowledge job timed out".into()),
                    };
                }
                thread::sleep(Duration::from_millis(40));
            }
            Err(e) => {
                return KbJobResult {
                    ok: false,
                    exit_code: -1,
                    result: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    error: Some(format!("wait: {e}")),
                };
            }
        }
    }

    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);

    let mut result = None;
    for line in stdout.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if v.is_object() {
                result = Some(v);
                break;
            }
        }
    }

    let ok = code == 0
        && result
            .as_ref()
            .and_then(|v| v.get("ok"))
            .and_then(|x| x.as_bool())
            .unwrap_or(code == 0);

    let error = if ok {
        None
    } else {
        result
            .as_ref()
            .and_then(|v| v.get("error"))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                if !stderr.trim().is_empty() {
                    Some(stderr.chars().take(500).collect())
                } else {
                    Some(format!("exit {code}"))
                }
            })
    };

    KbJobResult {
        ok,
        exit_code: code,
        result,
        stdout,
        stderr,
        error,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbRunRequest {
    pub action: String,
    #[serde(default)]
    pub payload: Value,
    pub timeout_secs: Option<u64>,
}

#[tauri::command]
pub fn kb_run(app: AppHandle, req: KbRunRequest) -> Result<KbJobResult, String> {
    let mut payload = if req.payload.is_null() {
        json!({})
    } else {
        req.payload
    };
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("action".into(), json!(req.action));
    }
    let timeout = req.timeout_secs.unwrap_or(120);
    Ok(run_kb(&app, payload, timeout))
}

/// Auto-ops binding table (profile ↔ knowledge bases).
pub fn ensure_auto_ops_table(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS cdp_auto_ops_binding (
            profile_id TEXT PRIMARY KEY NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 0,
            system_prompt TEXT NOT NULL DEFAULT '',
            kb_base_ids_json TEXT NOT NULL DEFAULT '[]',
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS xhs_auto_ops_logs (
            id TEXT PRIMARY KEY NOT NULL,
            profile_id TEXT NOT NULL,
            channel TEXT NOT NULL,
            tag TEXT NOT NULL DEFAULT '',
            tone TEXT NOT NULL DEFAULT 'neutral',
            message TEXT NOT NULL,
            time_label TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_xhs_auto_ops_logs_profile_ch
            ON xhs_auto_ops_logs(profile_id, channel, created_at DESC);
        CREATE TABLE IF NOT EXISTS xhs_acq_reply_history (
            id TEXT PRIMARY KEY NOT NULL,
            profile_id TEXT NOT NULL,
            search_keyword TEXT NOT NULL DEFAULT '',
            comment_keyword TEXT NOT NULL DEFAULT '',
            author_name TEXT NOT NULL DEFAULT '',
            comment_text TEXT NOT NULL DEFAULT '',
            reply_text TEXT NOT NULL DEFAULT '',
            note_title TEXT NOT NULL DEFAULT '',
            note_index INTEGER NOT NULL DEFAULT 0,
            ok INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_xhs_acq_reply_history_profile
            ON xhs_acq_reply_history(profile_id, created_at DESC);
        "#,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoOpsBinding {
    pub profile_id: String,
    pub enabled: bool,
    pub system_prompt: String,
    pub kb_base_ids_json: String,
    pub updated_at: i64,
}

#[tauri::command]
pub fn auto_ops_get_binding(
    db: tauri::State<'_, crate::library::LibraryState>,
    profile_id: String,
) -> Result<Option<AutoOpsBinding>, String> {
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    ensure_auto_ops_table(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT profile_id, enabled, system_prompt, kb_base_ids_json, updated_at \
             FROM cdp_auto_ops_binding WHERE profile_id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let row = stmt
        .query_row(rusqlite::params![profile_id], |r| {
            Ok(AutoOpsBinding {
                profile_id: r.get(0)?,
                enabled: r.get::<_, i64>(1)? != 0,
                system_prompt: r.get(2)?,
                kb_base_ids_json: r.get(3)?,
                updated_at: r.get(4)?,
            })
        })
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(row)
}

#[tauri::command]
pub fn auto_ops_upsert_binding(
    db: tauri::State<'_, crate::library::LibraryState>,
    body: AutoOpsBinding,
) -> Result<AutoOpsBinding, String> {
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    ensure_auto_ops_table(&conn)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    conn.execute(
        "INSERT INTO cdp_auto_ops_binding (profile_id, enabled, system_prompt, kb_base_ids_json, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(profile_id) DO UPDATE SET \
           enabled=excluded.enabled, \
           system_prompt=excluded.system_prompt, \
           kb_base_ids_json=excluded.kb_base_ids_json, \
           updated_at=excluded.updated_at",
        rusqlite::params![
            body.profile_id,
            if body.enabled { 1 } else { 0 },
            body.system_prompt,
            body.kb_base_ids_json,
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(AutoOpsBinding {
        profile_id: body.profile_id,
        enabled: body.enabled,
        system_prompt: body.system_prompt,
        kb_base_ids_json: body.kb_base_ids_json,
        updated_at: now,
    })
}

/// AI 客服会话 / 消息落库（弹窗会话记录面板用）
pub fn ensure_cs_chat_tables(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS xhs_cs_sessions (
            id TEXT PRIMARY KEY NOT NULL,
            profile_id TEXT NOT NULL,
            peer_name TEXT NOT NULL,
            last_preview TEXT NOT NULL DEFAULT '',
            message_count INTEGER NOT NULL DEFAULT 0,
            unread_badge INTEGER NOT NULL DEFAULT 0,
            last_direction TEXT NOT NULL DEFAULT '',
            updated_at INTEGER NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_xhs_cs_sessions_profile
            ON xhs_cs_sessions(profile_id, updated_at DESC);
        CREATE TABLE IF NOT EXISTS xhs_cs_messages (
            id TEXT PRIMARY KEY NOT NULL,
            session_id TEXT NOT NULL,
            profile_id TEXT NOT NULL,
            direction TEXT NOT NULL,
            text TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'auto',
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_xhs_cs_messages_session
            ON xhs_cs_messages(session_id, created_at ASC);
        "#,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn cs_session_id(profile_id: &str, peer_name: &str) -> String {
    let peer = peer_name.trim();
    let peer = if peer.is_empty() { "未知用户" } else { peer };
    format!("cs_{profile_id}_{peer}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsSession {
    pub id: String,
    pub profile_id: String,
    pub peer_name: String,
    pub last_preview: String,
    pub message_count: i64,
    pub unread_badge: i64,
    pub last_direction: String,
    pub updated_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsMessage {
    pub id: String,
    pub session_id: String,
    pub profile_id: String,
    pub direction: String,
    pub text: String,
    pub source: String,
    pub created_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsAppendBody {
    pub profile_id: String,
    pub peer_name: String,
    pub direction: String,
    pub text: String,
    pub source: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsSyncMessagesBody {
    pub profile_id: String,
    pub peer_name: String,
    pub messages: Vec<CsSyncMsgItem>,
    pub unread_badge: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsSyncMsgItem {
    pub direction: String,
    pub text: String,
}

fn map_cs_session(r: &rusqlite::Row<'_>) -> rusqlite::Result<CsSession> {
    Ok(CsSession {
        id: r.get(0)?,
        profile_id: r.get(1)?,
        peer_name: r.get(2)?,
        last_preview: r.get(3)?,
        message_count: r.get(4)?,
        unread_badge: r.get(5)?,
        last_direction: r.get(6)?,
        updated_at: r.get(7)?,
        created_at: r.get(8)?,
    })
}

fn map_cs_message(r: &rusqlite::Row<'_>) -> rusqlite::Result<CsMessage> {
    Ok(CsMessage {
        id: r.get(0)?,
        session_id: r.get(1)?,
        profile_id: r.get(2)?,
        direction: r.get(3)?,
        text: r.get(4)?,
        source: r.get(5)?,
        created_at: r.get(6)?,
    })
}

fn ensure_cs_session(
    conn: &rusqlite::Connection,
    profile_id: &str,
    peer_name: &str,
) -> Result<String, String> {
    let peer = {
        let t = peer_name.trim();
        if t.is_empty() {
            "未知用户".to_string()
        } else {
            t.to_string()
        }
    };
    let id = cs_session_id(profile_id, &peer);
    let now = now_ms();
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM xhs_cs_sessions WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if exists == 0 {
        conn.execute(
            "INSERT INTO xhs_cs_sessions
             (id, profile_id, peer_name, last_preview, message_count, unread_badge, last_direction, updated_at, created_at)
             VALUES (?1, ?2, ?3, '', 0, 0, '', ?4, ?4)",
            rusqlite::params![id, profile_id, peer, now],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "UPDATE xhs_cs_sessions SET peer_name = ?2 WHERE id = ?1",
            rusqlite::params![id, peer],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(id)
}

#[tauri::command]
pub fn cs_session_list(
    db: tauri::State<'_, crate::library::LibraryState>,
    profile_id: String,
) -> Result<Vec<CsSession>, String> {
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    ensure_cs_chat_tables(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, profile_id, peer_name, last_preview, message_count, unread_badge,
                    last_direction, updated_at, created_at
             FROM xhs_cs_sessions
             WHERE profile_id = ?1 AND message_count > 0
             ORDER BY updated_at DESC
             LIMIT 200",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![profile_id.trim()], map_cs_session)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn cs_message_list(
    db: tauri::State<'_, crate::library::LibraryState>,
    session_id: String,
    limit: Option<i64>,
) -> Result<Vec<CsMessage>, String> {
    let lim = limit.unwrap_or(300).clamp(1, 1000);
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    ensure_cs_chat_tables(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, profile_id, direction, text, source, created_at
             FROM xhs_cs_messages
             WHERE session_id = ?1
             ORDER BY created_at ASC
             LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![session_id.trim(), lim], map_cs_message)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn cs_message_append(
    db: tauri::State<'_, crate::library::LibraryState>,
    body: CsAppendBody,
) -> Result<CsMessage, String> {
    let text = body.text.trim().to_string();
    if text.is_empty() {
        return Err("消息内容为空".into());
    }
    let direction = match body.direction.trim().to_ascii_lowercase().as_str() {
        "sent" | "send" | "out" => "sent",
        _ => "recv",
    }
    .to_string();
    let source = body
        .source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("auto")
        .to_string();
    let profile_id = body.profile_id.trim().to_string();
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    ensure_cs_chat_tables(&conn)?;
    let session_id = ensure_cs_session(&conn, &profile_id, &body.peer_name)?;
    let now = now_ms();
    let id = format!("cm_{now}_{}", direction.chars().next().unwrap_or('x'));
    // 简单去重：同会话同方向同文案在 2s 内不重复写入
    let dup: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM xhs_cs_messages
             WHERE session_id = ?1 AND direction = ?2 AND text = ?3 AND created_at > ?4",
            rusqlite::params![session_id, direction, text, now - 2000],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if dup > 0 {
        let existing = conn
            .query_row(
                "SELECT id, session_id, profile_id, direction, text, source, created_at
                 FROM xhs_cs_messages
                 WHERE session_id = ?1 AND direction = ?2 AND text = ?3
                 ORDER BY created_at DESC LIMIT 1",
                rusqlite::params![session_id, direction, text],
                map_cs_message,
            )
            .map_err(|e| e.to_string())?;
        return Ok(existing);
    }
    conn.execute(
        "INSERT INTO xhs_cs_messages (id, session_id, profile_id, direction, text, source, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![id, session_id, profile_id, direction, text, source, now],
    )
    .map_err(|e| e.to_string())?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM xhs_cs_messages WHERE session_id = ?1",
            rusqlite::params![session_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let preview: String = text.chars().take(80).collect();
    conn.execute(
        "UPDATE xhs_cs_sessions SET last_preview = ?2, message_count = ?3, last_direction = ?4, updated_at = ?5
         WHERE id = ?1",
        rusqlite::params![session_id, preview, count, direction, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(CsMessage {
        id,
        session_id,
        profile_id,
        direction,
        text,
        source,
        created_at: now,
    })
}

/// 用浏览器同步到的整段对话覆盖式合并（按文案去重追加）
#[tauri::command]
pub fn cs_messages_sync(
    db: tauri::State<'_, crate::library::LibraryState>,
    body: CsSyncMessagesBody,
) -> Result<CsSession, String> {
    let profile_id = body.profile_id.trim().to_string();
    let peer = body.peer_name.trim().to_string();
    if peer.is_empty() {
        return Err("用户名为空".into());
    }
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    ensure_cs_chat_tables(&conn)?;
    let session_id = ensure_cs_session(&conn, &profile_id, &peer)?;
    let mut last_dir = String::new();
    let mut last_text = String::new();
    for (i, m) in body.messages.iter().enumerate() {
        let text = m.text.trim();
        if text.is_empty() {
            continue;
        }
        let direction = match m.direction.trim().to_ascii_lowercase().as_str() {
            "sent" | "send" | "out" => "sent",
            _ => "recv",
        };
        last_dir = direction.to_string();
        last_text = text.to_string();
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM xhs_cs_messages
                 WHERE session_id = ?1 AND direction = ?2 AND text = ?3",
                rusqlite::params![session_id, direction, text],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if exists > 0 {
            continue;
        }
        let now = now_ms() + i as i64;
        let id = format!("cm_{now}_{i}");
        conn.execute(
            "INSERT INTO xhs_cs_messages (id, session_id, profile_id, direction, text, source, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'sync', ?6)",
            rusqlite::params![id, session_id, profile_id, direction, text, now],
        )
        .map_err(|e| e.to_string())?;
    }
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM xhs_cs_messages WHERE session_id = ?1",
            rusqlite::params![session_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let now = now_ms();
    let preview: String = last_text.chars().take(80).collect();
    let badge = body.unread_badge.unwrap_or(0);
    conn.execute(
        "UPDATE xhs_cs_sessions SET last_preview = CASE WHEN ?2 = '' THEN last_preview ELSE ?2 END,
             message_count = ?3, last_direction = CASE WHEN ?4 = '' THEN last_direction ELSE ?4 END,
             unread_badge = ?5, updated_at = ?6
         WHERE id = ?1",
        rusqlite::params![session_id, preview, count, last_dir, badge, now],
    )
    .map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT id, profile_id, peer_name, last_preview, message_count, unread_badge,
                last_direction, updated_at, created_at
         FROM xhs_cs_sessions WHERE id = ?1",
        rusqlite::params![session_id],
        map_cs_session,
    )
    .map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsTouchSessionBody {
    pub profile_id: String,
    pub peer_name: String,
    pub last_preview: Option<String>,
    pub unread_badge: Option<i64>,
}

#[tauri::command]
pub fn cs_session_touch(
    db: tauri::State<'_, crate::library::LibraryState>,
    body: CsTouchSessionBody,
) -> Result<CsSession, String> {
    let profile_id = body.profile_id.trim().to_string();
    let peer = body.peer_name.trim().to_string();
    if peer.is_empty() {
        return Err("用户名为空".into());
    }
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    ensure_cs_chat_tables(&conn)?;
    let session_id = ensure_cs_session(&conn, &profile_id, &peer)?;
    let now = now_ms();
    let preview = body
        .last_preview
        .as_deref()
        .unwrap_or("")
        .chars()
        .take(80)
        .collect::<String>();
    let badge = body.unread_badge.unwrap_or(0);
    if !preview.is_empty() {
        conn.execute(
            "UPDATE xhs_cs_sessions SET last_preview = ?2, unread_badge = ?3, updated_at = ?4 WHERE id = ?1",
            rusqlite::params![session_id, preview, badge, now],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "UPDATE xhs_cs_sessions SET unread_badge = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![session_id, badge, now],
        )
        .map_err(|e| e.to_string())?;
    }
    conn.query_row(
        "SELECT id, profile_id, peer_name, last_preview, message_count, unread_badge,
                last_direction, updated_at, created_at
         FROM xhs_cs_sessions WHERE id = ?1",
        rusqlite::params![session_id],
        map_cs_session,
    )
    .map_err(|e| e.to_string())
}

/// AI 客服 / AI 获客 控制台执行日志
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoOpsConsoleLog {
    pub id: String,
    pub profile_id: String,
    pub channel: String,
    pub tag: String,
    pub tone: String,
    pub message: String,
    pub time_label: String,
    pub created_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoOpsLogAppendBody {
    pub profile_id: String,
    /// `cs` | `acq`
    pub channel: String,
    pub tag: Option<String>,
    pub tone: Option<String>,
    pub message: String,
    pub time_label: Option<String>,
}

fn normalize_ops_channel(raw: &str) -> Result<&'static str, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "cs" | "客服" => Ok("cs"),
        "acq" | "获客" => Ok("acq"),
        other => Err(format!("未知日志频道: {other}")),
    }
}

fn map_auto_ops_log(r: &rusqlite::Row<'_>) -> rusqlite::Result<AutoOpsConsoleLog> {
    Ok(AutoOpsConsoleLog {
        id: r.get(0)?,
        profile_id: r.get(1)?,
        channel: r.get(2)?,
        tag: r.get(3)?,
        tone: r.get(4)?,
        message: r.get(5)?,
        time_label: r.get(6)?,
        created_at: r.get(7)?,
    })
}

fn prune_auto_ops_logs(conn: &rusqlite::Connection, profile_id: &str, channel: &str) {
    const KEEP: i64 = 500;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM xhs_auto_ops_logs WHERE profile_id = ?1 AND channel = ?2",
            rusqlite::params![profile_id, channel],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if count <= KEEP {
        return;
    }
    let drop_n = count - KEEP;
    let _ = conn.execute(
        "DELETE FROM xhs_auto_ops_logs WHERE id IN (
            SELECT id FROM xhs_auto_ops_logs
            WHERE profile_id = ?1 AND channel = ?2
            ORDER BY created_at ASC
            LIMIT ?3
        )",
        rusqlite::params![profile_id, channel, drop_n],
    );
}

#[tauri::command]
pub fn auto_ops_log_append(
    db: tauri::State<'_, crate::library::LibraryState>,
    body: AutoOpsLogAppendBody,
) -> Result<AutoOpsConsoleLog, String> {
    let profile_id = body.profile_id.trim().to_string();
    if profile_id.is_empty() {
        return Err("profileId 为空".into());
    }
    let channel = normalize_ops_channel(&body.channel)?.to_string();
    let message = body.message.trim().to_string();
    if message.is_empty() {
        return Err("日志内容为空".into());
    }
    let tag = body
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("系统")
        .to_string();
    let tone = body
        .tone
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("neutral")
        .to_string();
    let time_label = body
        .time_label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_default();

    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    ensure_auto_ops_table(&conn)?;
    let now = now_ms();
    static LOG_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = LOG_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let id = format!("aol_{channel}_{now}_{seq}");
    conn.execute(
        "INSERT INTO xhs_auto_ops_logs
         (id, profile_id, channel, tag, tone, message, time_label, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            id,
            profile_id,
            channel,
            tag,
            tone,
            message,
            time_label,
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    prune_auto_ops_logs(&conn, &profile_id, &channel);
    Ok(AutoOpsConsoleLog {
        id,
        profile_id,
        channel,
        tag,
        tone,
        message,
        time_label,
        created_at: now,
    })
}

#[tauri::command]
pub fn auto_ops_log_list(
    db: tauri::State<'_, crate::library::LibraryState>,
    profile_id: String,
    channel: String,
    limit: Option<i64>,
) -> Result<Vec<AutoOpsConsoleLog>, String> {
    let profile_id = profile_id.trim().to_string();
    if profile_id.is_empty() {
        return Ok(vec![]);
    }
    let channel = normalize_ops_channel(&channel)?.to_string();
    let lim = limit.unwrap_or(200).clamp(1, 1000);
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    ensure_auto_ops_table(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, profile_id, channel, tag, tone, message, time_label, created_at
             FROM xhs_auto_ops_logs
             WHERE profile_id = ?1 AND channel = ?2
             ORDER BY created_at DESC
             LIMIT ?3",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![profile_id, channel, lim], map_auto_ops_log)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    out.reverse();
    Ok(out)
}

/// 获客：已回复评论历史
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcqReplyHistoryRow {
    pub id: String,
    pub profile_id: String,
    pub search_keyword: String,
    pub comment_keyword: String,
    pub author_name: String,
    pub comment_text: String,
    pub reply_text: String,
    pub note_title: String,
    pub note_index: i64,
    pub ok: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct AcqReplyHistoryInsert {
    pub profile_id: String,
    pub search_keyword: String,
    pub comment_keyword: String,
    pub author_name: String,
    pub comment_text: String,
    pub reply_text: String,
    pub note_title: String,
    pub note_index: i64,
    pub ok: bool,
}

fn prune_acq_reply_history(conn: &rusqlite::Connection, profile_id: &str) {
    const KEEP: i64 = 500;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM xhs_acq_reply_history WHERE profile_id = ?1",
            rusqlite::params![profile_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if count <= KEEP {
        return;
    }
    let drop_n = count - KEEP;
    let _ = conn.execute(
        "DELETE FROM xhs_acq_reply_history WHERE id IN (
            SELECT id FROM xhs_acq_reply_history
            WHERE profile_id = ?1
            ORDER BY created_at ASC
            LIMIT ?2
        )",
        rusqlite::params![profile_id, drop_n],
    );
}

/// 后台获客线程写入一条回复记录（失败忽略，不阻断 CDP）
pub fn acq_reply_history_insert_quiet(
    db: &crate::library::LibraryState,
    row: AcqReplyHistoryInsert,
) {
    let Ok(conn) = db.conn.lock() else {
        return;
    };
    if ensure_auto_ops_table(&conn).is_err() {
        return;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let id = format!("acq_r_{nanos}");
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let _ = conn.execute(
        "INSERT INTO xhs_acq_reply_history
         (id, profile_id, search_keyword, comment_keyword, author_name, comment_text,
          reply_text, note_title, note_index, ok, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        rusqlite::params![
            id,
            row.profile_id.trim(),
            row.search_keyword,
            row.comment_keyword,
            row.author_name,
            row.comment_text,
            row.reply_text,
            row.note_title,
            row.note_index,
            if row.ok { 1 } else { 0 },
            created_at
        ],
    );
    prune_acq_reply_history(&conn, row.profile_id.trim());
}

#[tauri::command]
pub fn acq_reply_history_list(
    db: tauri::State<'_, crate::library::LibraryState>,
    profile_id: String,
    limit: Option<i64>,
) -> Result<Vec<AcqReplyHistoryRow>, String> {
    let profile_id = profile_id.trim().to_string();
    if profile_id.is_empty() {
        return Ok(vec![]);
    }
    let lim = limit.unwrap_or(100).clamp(1, 500);
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    ensure_auto_ops_table(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, profile_id, search_keyword, comment_keyword, author_name, comment_text,
                    reply_text, note_title, note_index, ok, created_at
             FROM xhs_acq_reply_history
             WHERE profile_id = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![profile_id, lim], |r| {
            Ok(AcqReplyHistoryRow {
                id: r.get(0)?,
                profile_id: r.get(1)?,
                search_keyword: r.get(2)?,
                comment_keyword: r.get(3)?,
                author_name: r.get(4)?,
                comment_text: r.get(5)?,
                reply_text: r.get(6)?,
                note_title: r.get(7)?,
                note_index: r.get(8)?,
                ok: r.get::<_, i64>(9)? != 0,
                created_at: r.get(10)?,
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
pub fn acq_reply_history_clear(
    db: tauri::State<'_, crate::library::LibraryState>,
    profile_id: String,
) -> Result<serde_json::Value, String> {
    let profile_id = profile_id.trim().to_string();
    if profile_id.is_empty() {
        return Err("profileId 为空".into());
    }
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    ensure_auto_ops_table(&conn)?;
    let n = conn
        .execute(
            "DELETE FROM xhs_acq_reply_history WHERE profile_id = ?1",
            rusqlite::params![profile_id],
        )
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "deleted": n }))
}
