//! Spawn Node scripts under resources/ai and resources/cdp.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use crate::catbus::resolve_node_bin;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeJobResult {
    pub ok: bool,
    pub exit_code: i32,
    pub result: Option<Value>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CdpProgressEvent {
    pub step: String,
    pub draft_id: String,
    pub action: String,
}

fn resource_subdir(app: &AppHandle, name: &str) -> PathBuf {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../resources/{name}")),
        app.path()
            .resource_dir()
            .map(|p| p.join(format!("resources/{name}")))
            .unwrap_or_default(),
        app.path()
            .resource_dir()
            .map(|p| p.join(name))
            .unwrap_or_default(),
    ];
    for c in candidates {
        if c.join("run.mjs").is_file() {
            return c.canonicalize().unwrap_or(c);
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../resources/{name}"))
}

struct ProgressCtx {
    draft_id: String,
    action: String,
}

fn run_script(
    app: &AppHandle,
    subdir: &str,
    args: &[String],
    stdin_json: Option<&str>,
    timeout_secs: u64,
    progress: Option<ProgressCtx>,
) -> NodeJobResult {
    let dir = resource_subdir(app, subdir);
    let entry = dir.join("run.mjs");
    if !entry.is_file() {
        return NodeJobResult {
            ok: false,
            exit_code: -1,
            result: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(format!(
                "missing {}. Run npm install under resources/{subdir}",
                entry.display()
            )),
        };
    }
    let node = match resolve_node_bin(app) {
        Ok(n) => n,
        Err(e) => {
            return NodeJobResult {
                ok: false,
                exit_code: -1,
                result: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(e),
            };
        }
    };

    let mut cmd = Command::new(&node);
    cmd.arg(&entry)
        .args(args)
        .current_dir(&dir)
        .env("STAFF_NODE_BIN", &node)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return NodeJobResult {
                ok: false,
                exit_code: -1,
                result: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(format!("spawn: {e}")),
            };
        }
    };

    if let Some(text) = stdin_json {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
    } else {
        drop(child.stdin.take());
    }

    let stdout_pipe = child.stdout.take();
    let app_progress = app.clone();
    let progress_draft = progress
        .as_ref()
        .map(|p| p.draft_id.clone())
        .unwrap_or_default();
    let progress_action = progress
        .as_ref()
        .map(|p| p.action.clone())
        .unwrap_or_default();
    let (tx, rx) = mpsc::channel::<String>();
    let stdout_thread = thread::spawn(move || {
        let mut collected = String::new();
        let Some(out) = stdout_pipe else {
            let _ = tx.send(collected);
            return;
        };
        let mut reader = BufReader::new(out);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                        if v.get("__cdpProgress")
                            .and_then(|x| x.as_bool())
                            .unwrap_or(false)
                        {
                            let step = v
                                .get("step")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string();
                            let draft_id = v
                                .get("draftId")
                                .and_then(|x| x.as_str())
                                .unwrap_or(&progress_draft)
                                .to_string();
                            if !step.is_empty() {
                                let _ = app_progress.emit(
                                    "cdp-progress",
                                    CdpProgressEvent {
                                        step,
                                        draft_id,
                                        action: progress_action.clone(),
                                    },
                                );
                            }
                            continue;
                        }
                    }
                    collected.push_str(&line);
                    if !line.ends_with('\n') {
                        collected.push('\n');
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(collected);
    });

    let stderr_pipe = child.stderr.take();
    let (tx_err, rx_err) = mpsc::channel::<String>();
    let stderr_thread = thread::spawn(move || {
        let mut stderr = String::new();
        if let Some(mut err) = stderr_pipe {
            let _ = err.read_to_string(&mut stderr);
        }
        let _ = tx_err.send(stderr);
    });

    let timeout = Duration::from_secs(timeout_secs.max(1));
    let start = std::time::Instant::now();
    let mut timed_out = false;
    let mut code: i32 = -1;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                code = status.code().unwrap_or(-1);
                break;
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    timed_out = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                thread::sleep(Duration::from_millis(40));
            }
            Err(e) => {
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return NodeJobResult {
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

    let stdout = rx.recv().unwrap_or_default();
    let stderr = rx_err.recv().unwrap_or_default();
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    if timed_out {
        return NodeJobResult {
            ok: false,
            exit_code: -1,
            result: None,
            stdout,
            stderr,
            error: Some("node job timed out".into()),
        };
    }

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
                    Some(stderr.chars().take(400).collect())
                } else {
                    Some(format!("exit {code}"))
                }
            })
    };

    NodeJobResult {
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
pub struct AiRunRequest {
    pub action: String,
    pub payload: Value,
    pub timeout_secs: Option<u64>,
}

/// 供后台图片任务等同步调用（勿在 UI 线程直接跑）
pub fn run_ai_action(
    app: &AppHandle,
    action: &str,
    payload: &Value,
    timeout_secs: u64,
) -> NodeJobResult {
    let stdin = match serde_json::to_string(payload) {
        Ok(s) => s,
        Err(e) => {
            return NodeJobResult {
                ok: false,
                exit_code: -1,
                result: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(format!("serialize payload: {e}")),
            }
        }
    };
    run_script(
        app,
        "ai",
        &[action.to_string()],
        Some(&stdin),
        timeout_secs,
        None,
    )
}

#[tauri::command]
pub async fn ai_run(app: AppHandle, req: AiRunRequest) -> Result<NodeJobResult, String> {
    let timeout = req.timeout_secs.unwrap_or(120);
    let action = req.action;
    let payload = req.payload;
    tauri::async_runtime::spawn_blocking(move || run_ai_action(&app, &action, &payload, timeout))
        .await
        .map_err(|e| format!("ai_run join: {e}"))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CdpRunRequest {
    pub action: String,
    pub payload: Value,
    pub timeout_secs: Option<u64>,
}

#[tauri::command]
pub async fn cdp_run(app: AppHandle, req: CdpRunRequest) -> Result<NodeJobResult, String> {
    let stdin = serde_json::to_string(&req.payload).map_err(|e| e.to_string())?;
    let timeout = req.timeout_secs.unwrap_or(300);
    let action = req.action.clone();
    let draft_id = req
        .payload
        .get("draftId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let progress = Some(ProgressCtx {
        draft_id,
        action: action.clone(),
    });
    tauri::async_runtime::spawn_blocking(move || {
        run_script(&app, "cdp", &[action], Some(&stdin), timeout, progress)
    })
    .await
    .map_err(|e| format!("cdp_run join: {e}"))
}
