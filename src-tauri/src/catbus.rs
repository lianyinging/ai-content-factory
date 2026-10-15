use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorInfo {
    pub ready: bool,
    pub node: Option<String>,
    pub cli: Option<String>,
    pub catbus_home: String,
    pub plugin_dir: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatbusRunRequest {
    pub args: Vec<String>,
    pub stdin: Option<String>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatbusRunResult {
    pub ok: bool,
    pub exit_code: i32,
    pub envelope: Option<Value>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    pub catbus_home: String,
}

struct QrJob {
    platform: String,
    account: String,
    status: String, // running | success | failed
    envelope: Option<Value>,
    error: Option<String>,
    child: Option<Child>,
}

pub struct CatbusState {
    qr: Mutex<Option<QrJob>>,
}

impl Default for CatbusState {
    fn default() -> Self {
        Self {
            qr: Mutex::new(None),
        }
    }
}

fn resource_catbus_dir(app: &AppHandle) -> PathBuf {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/catbus"),
        app.path()
            .resource_dir()
            .map(|p| p.join("resources/catbus"))
            .unwrap_or_default(),
        app.path()
            .resource_dir()
            .map(|p| p.join("catbus"))
            .unwrap_or_default(),
    ];
    for c in candidates {
        if resolve_cli_entry(&c).is_ok() {
            return c.canonicalize().unwrap_or(c);
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/catbus");
    dev.canonicalize().unwrap_or(dev)
}

pub fn catbus_home(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?
        .join("catbus");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir CATBUS_HOME: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

fn resolve_cli_entry(plugin_dir: &Path) -> Result<PathBuf, String> {
    let candidates = [
        plugin_dir.join("node_modules/catbus-cli/dist/cli/main.js"),
        plugin_dir.join("node_modules/catbus-cli/dist/cli/main.mjs"),
        plugin_dir.join("node_modules/catbus-cli/dist/cli.js"),
    ];
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
    }
    Err(format!(
        "catbus-cli missing under {}. Run: cd resources/catbus && npm install",
        plugin_dir.display()
    ))
}

pub fn resolve_node_bin(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(env) = std::env::var("STAFF_NODE_BIN") {
        let p = PathBuf::from(env.trim());
        if p.is_file() {
            return Ok(p);
        }
    }

    let triple = host_triple();
    let local_candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("binaries/node-{triple}")),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("binaries/node-{triple}.exe")),
    ];
    for c in local_candidates {
        if c.is_file() {
            return Ok(c);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in [
                format!("node-{triple}"),
                format!("node-{triple}.exe"),
                "node".to_string(),
                "node.exe".to_string(),
            ] {
                let p = dir.join(&name);
                if p.is_file() {
                    return Ok(p);
                }
                let res = dir.join("../Resources").join(&name);
                if res.is_file() {
                    return Ok(res.canonicalize().unwrap_or(res));
                }
            }
        }
    }

    // Avoid unused warning in release when app only used for resource paths above
    let _ = app;

    which_node().map_err(|_| {
        "Node not found. Run: npm run fetch-node (or set STAFF_NODE_BIN)".into()
    })
}

fn which_node() -> Result<PathBuf, String> {
    let output = Command::new("which")
        .arg("node")
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("node not in PATH".into());
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return Err("node not in PATH".into());
    }
    Ok(PathBuf::from(path))
}

fn host_triple() -> String {
    if let Ok(out) = Command::new("rustc")
        .args(["--print", "host-tuple"])
        .output()
    {
        if out.status.success() {
            let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !t.is_empty() {
                return t;
            }
        }
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return "aarch64-apple-darwin".into();
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return "x86_64-apple-darwin".into();
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return "x86_64-unknown-linux-gnu".into();
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return "x86_64-pc-windows-msvc".into();
    }
    #[allow(unreachable_code)]
    "unknown".into()
}

fn parse_envelope(stdout: &str) -> Option<Value> {
    let text = stdout.trim();
    if text.is_empty() {
        return None;
    }
    for line in text.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if v.is_object() {
                return Some(v);
            }
        }
    }
    serde_json::from_str::<Value>(text)
        .ok()
        .filter(|v| v.is_object())
}

fn run_catbus_inner(
    app: &AppHandle,
    args: &[String],
    stdin_text: Option<&str>,
    timeout_secs: u64,
) -> CatbusRunResult {
    let home = match catbus_home(app) {
        Ok(h) => h,
        Err(e) => {
            return CatbusRunResult {
                ok: false,
                exit_code: -1,
                envelope: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(e),
                catbus_home: String::new(),
            };
        }
    };
    let plugin = resource_catbus_dir(app);
    let node = match resolve_node_bin(app) {
        Ok(n) => n,
        Err(e) => {
            return CatbusRunResult {
                ok: false,
                exit_code: -1,
                envelope: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(e),
                catbus_home: home.display().to_string(),
            };
        }
    };
    let entry = match resolve_cli_entry(&plugin) {
        Ok(e) => e,
        Err(e) => {
            return CatbusRunResult {
                ok: false,
                exit_code: -1,
                envelope: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(e),
                catbus_home: home.display().to_string(),
            };
        }
    };

    let mut cmd = Command::new(&node);
    cmd.arg(&entry)
        .args(args)
        .current_dir(&plugin)
        .env("CATBUS_HOME", &home)
        .env("STAFF_NODE_BIN", &node)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return CatbusRunResult {
                ok: false,
                exit_code: -1,
                envelope: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(format!("spawn node: {e}")),
                catbus_home: home.display().to_string(),
            };
        }
    };

    if let Some(text) = stdin_text {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
    } else {
        drop(child.stdin.take());
    }

    let timeout = Duration::from_secs(timeout_secs.max(1));
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return CatbusRunResult {
                        ok: false,
                        exit_code: -1,
                        envelope: None,
                        stdout: String::new(),
                        stderr: String::new(),
                        error: Some("catbus timed out".into()),
                        catbus_home: home.display().to_string(),
                    };
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return CatbusRunResult {
                    ok: false,
                    exit_code: -1,
                    envelope: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    error: Some(format!("wait: {e}")),
                    catbus_home: home.display().to_string(),
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
    let status = child.wait().ok();
    let exit_code = status.and_then(|s| s.code()).unwrap_or(-1);
    let envelope = parse_envelope(&stdout);
    let ok = envelope
        .as_ref()
        .and_then(|e| e.get("ok"))
        .and_then(|v| v.as_bool())
        .unwrap_or(exit_code == 0);
    let error = if ok {
        None
    } else {
        let msg = envelope.as_ref().and_then(|e| e.get("error")).and_then(|err| {
            err.get("message")
                .or_else(|| err.get("code"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });
        Some(msg.unwrap_or_else(|| {
            let s = stderr.trim();
            if s.is_empty() {
                format!("exit {exit_code}")
            } else {
                s.chars().take(800).collect()
            }
        }))
    };

    CatbusRunResult {
        ok,
        exit_code,
        envelope,
        stdout: stdout.trim().to_string(),
        stderr: stderr.trim().to_string(),
        error,
        catbus_home: home.display().to_string(),
    }
}

fn result_payload(result: CatbusRunResult) -> Value {
    if let Some(mut envelope) = result.envelope.clone() {
        if let Some(obj) = envelope.as_object_mut() {
            obj.insert(
                "_meta".into(),
                serde_json::json!({
                    "exitCode": result.exit_code,
                    "catbusHome": result.catbus_home,
                    "stderr": result.stderr.chars().take(400).collect::<String>(),
                }),
            );
        }
        return envelope;
    }
    serde_json::json!({
        "ok": result.ok,
        "data": null,
        "page": null,
        "error": {
            "code": "ERROR",
            "message": result.error.unwrap_or_else(|| "catbus failed".into()),
            "hint": "检查 Node sidecar 与 resources/catbus 是否已 npm install",
            "detail": result.stderr.chars().take(800).collect::<String>(),
        },
        "_meta": {
            "exitCode": result.exit_code,
            "catbusHome": result.catbus_home,
        }
    })
}

pub fn qrcode_path(app: &AppHandle, platform: &str) -> Result<PathBuf, String> {
    Ok(catbus_home(app)?
        .join("cache")
        .join(platform)
        .join("qrcode.png"))
}

pub fn normalize_platform(raw: &str) -> Result<String, String> {
    let key = raw.trim().to_lowercase();
    let key = match key.as_str() {
        "xiaohongshu" | "rednote" => "xhs",
        "dy" => "douyin",
        other => other,
    };
    match key {
        "xhs" | "douyin" => Ok(key.into()),
        _ => Err(format!("unsupported platform: {raw}")),
    }
}

fn poll_qr_child(job: &mut QrJob) {
    if job.status != "running" {
        return;
    }
    let Some(child) = job.child.as_mut() else {
        return;
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            let mut stdout = String::new();
            let mut stderr = String::new();
            if let Some(mut out) = child.stdout.take() {
                let _ = out.read_to_string(&mut stdout);
            }
            if let Some(mut err) = child.stderr.take() {
                let _ = err.read_to_string(&mut stderr);
            }
            let _ = child.wait();
            job.child = None;
            let envelope = parse_envelope(&stdout);
            let ok = envelope
                .as_ref()
                .and_then(|e| e.get("ok"))
                .and_then(|v| v.as_bool())
                .unwrap_or(status.success());
            job.envelope = envelope.clone();
            if ok {
                job.status = "success".into();
            } else {
                job.status = "failed".into();
                job.error = envelope
                    .as_ref()
                    .and_then(|e| e.get("error"))
                    .and_then(|err| {
                        err.get("message")
                            .or_else(|| err.get("code"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
                    .or_else(|| {
                        let s = stderr.trim();
                        if s.is_empty() {
                            None
                        } else {
                            Some(s.chars().take(800).collect())
                        }
                    })
                    .or_else(|| Some(format!("exit {:?}", status.code())));
            }
        }
        Ok(None) => {}
        Err(e) => {
            job.status = "failed".into();
            job.error = Some(e.to_string());
            job.child = None;
        }
    }
}

#[tauri::command]
pub fn catbus_doctor(app: AppHandle) -> DoctorInfo {
    let home = catbus_home(&app)
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let plugin = resource_catbus_dir(&app);
    let plugin_s = plugin.display().to_string();
    let mut info = DoctorInfo {
        ready: false,
        node: None,
        cli: None,
        catbus_home: home,
        plugin_dir: plugin_s,
        error: None,
    };
    match resolve_node_bin(&app) {
        Ok(n) => info.node = Some(n.display().to_string()),
        Err(e) => {
            info.error = Some(e);
            return info;
        }
    }
    match resolve_cli_entry(&plugin) {
        Ok(c) => {
            info.cli = Some(c.display().to_string());
            info.ready = true;
        }
        Err(e) => info.error = Some(e),
    }
    info
}

#[tauri::command]
pub fn catbus_run(app: AppHandle, req: CatbusRunRequest) -> Value {
    let timeout = req.timeout_secs.unwrap_or(180);
    let result = run_catbus_inner(&app, &req.args, req.stdin.as_deref(), timeout);
    result_payload(result)
}

#[tauri::command]
pub fn catbus_qr_start(
    app: AppHandle,
    state: tauri::State<'_, CatbusState>,
    platform: String,
    account: String,
) -> Result<Value, String> {
    let plat = normalize_platform(&platform)?;
    let acct = if account.trim().is_empty() {
        "default".into()
    } else {
        account.trim().to_string()
    };

    let home = catbus_home(&app)?;
    let plugin = resource_catbus_dir(&app);
    let node = resolve_node_bin(&app)?;
    let entry = resolve_cli_entry(&plugin)?;

    let mut guard = state.qr.lock().map_err(|e| e.to_string())?;
    if let Some(old) = guard.as_mut() {
        if let Some(mut child) = old.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    let mut cmd = Command::new(&node);
    cmd.arg(&entry)
        .args([
            plat.as_str(),
            "auth",
            "login",
            "--method",
            "qrcode",
            "-a",
            &acct,
        ])
        .current_dir(&plugin)
        .env("CATBUS_HOME", &home)
        .env("STAFF_NODE_BIN", &node)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = cmd.spawn().map_err(|e| format!("spawn qr login: {e}"))?;

    *guard = Some(QrJob {
        platform: plat.clone(),
        account: acct.clone(),
        status: "running".into(),
        envelope: None,
        error: None,
        child: Some(child),
    });

    let has_qr = qrcode_path(&app, &plat).map(|p| p.is_file()).unwrap_or(false);
    Ok(serde_json::json!({
        "ok": true,
        "status": "running",
        "platform": plat,
        "account": acct,
        "hasQr": has_qr,
    }))
}

#[tauri::command]
pub fn catbus_qr_status(
    app: AppHandle,
    state: tauri::State<'_, CatbusState>,
    platform: String,
) -> Result<Value, String> {
    let plat = normalize_platform(&platform)?;
    let mut guard = state.qr.lock().map_err(|e| e.to_string())?;
    let Some(job) = guard.as_mut() else {
        return Ok(serde_json::json!({
            "ok": true,
            "status": "idle",
            "platform": plat,
            "account": null,
            "hasQr": qrcode_path(&app, &plat).map(|p| p.is_file()).unwrap_or(false),
        }));
    };
    if job.platform != plat {
        return Ok(serde_json::json!({
            "ok": true,
            "status": "idle",
            "platform": plat,
            "account": null,
            "hasQr": qrcode_path(&app, &plat).map(|p| p.is_file()).unwrap_or(false),
            "note": format!("active job is {}", job.platform),
        }));
    }
    poll_qr_child(job);
    let has_qr = qrcode_path(&app, &plat).map(|p| p.is_file()).unwrap_or(false);
    Ok(serde_json::json!({
        "ok": job.status == "success",
        "status": job.status,
        "platform": job.platform,
        "account": job.account,
        "error": job.error,
        "envelope": job.envelope,
        "hasQr": has_qr,
    }))
}

#[tauri::command]
pub fn catbus_qr_image(app: AppHandle, platform: String) -> Result<Value, String> {
    let plat = normalize_platform(&platform)?;
    let path = qrcode_path(&app, &plat)?;
    if !path.is_file() {
        return Ok(serde_json::json!({ "ok": false, "platform": plat }));
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(serde_json::json!({
        "ok": true,
        "platform": plat,
        "mime": "image/png",
        "base64": b64,
    }))
}
