//! Local Chromium CDP for AI Content Factory.
//! Multi-profile: each profile has its own user-data-dir + debug port.

use std::cell::Cell;
use std::collections::HashMap;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};


const DEFAULT_PORT: u16 = 9222;
const GLOBAL_ID: &str = "default";
const DEFAULT_PROFILE_ID: &str = "default";

pub struct BrowserProcess(pub Mutex<HashMap<String, Child>>);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserConfig {
    pub browser_executable: String,
    pub browser_kind: String,
    pub user_data_dir: String,
    pub debug_port: u16,
    pub auto_launch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserCandidate {
    pub kind: String,
    pub label: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserProfile {
    pub id: String,
    pub name: String,
    /// both | xhs | douyin
    pub platforms: String,
    pub user_data_dir: String,
    pub debug_port: u16,
    /// JSON: { xhs?: SessionInfo, douyin?: SessionInfo }
    pub session_json: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPlatformInfo {
    pub logged_in: bool,
    pub nickname: String,
    pub avatar_url: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub profile_id: String,
    pub profile_name: String,
    pub configured: bool,
    pub running: bool,
    pub cdp_online: bool,
    pub debug_port: u16,
    pub browser_executable: String,
    pub user_data_dir: String,
    pub browser_kind: String,
    pub auto_launch: bool,
    pub platforms: String,
    pub session_json: String,
    pub message: String,
    pub browser_version: Option<String>,
    pub web_socket_debugger_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatusSummary {
    pub configured: bool,
    pub online_count: usize,
    pub total_count: usize,
    pub profiles: Vec<BrowserStatus>,
    pub message: String,
    /// Backward-compat: first online or default profile flattened fields
    pub running: bool,
    pub cdp_online: bool,
    pub debug_port: u16,
    pub browser_executable: String,
    pub user_data_dir: String,
    pub browser_kind: String,
    pub auto_launch: bool,
    pub browser_version: Option<String>,
    pub web_socket_debugger_url: Option<String>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn ensure_browser_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS browser_automation (
            id TEXT PRIMARY KEY NOT NULL,
            browser_executable TEXT NOT NULL DEFAULT '',
            browser_kind TEXT NOT NULL DEFAULT 'chrome',
            user_data_dir TEXT NOT NULL DEFAULT '',
            debug_port INTEGER NOT NULL DEFAULT 9222,
            auto_launch INTEGER NOT NULL DEFAULT 1,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS browser_profile (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL DEFAULT '默认',
            platforms TEXT NOT NULL DEFAULT 'both',
            user_data_dir TEXT NOT NULL DEFAULT '',
            debug_port INTEGER NOT NULL DEFAULT 9222,
            session_json TEXT NOT NULL DEFAULT '{}',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        ",
    )
    .map_err(|e| format!("browser tables migrate: {e}"))?;

    // Migrate legacy single-row config into a default profile once.
    let profile_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM browser_profile", [], |r| r.get(0))
        .unwrap_or(0);
    if profile_count == 0 {
        let legacy = conn
            .query_row(
                "SELECT user_data_dir, debug_port FROM browser_automation WHERE id = ?1",
                params![GLOBAL_ID],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u16)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let (udir, port) = legacy.unwrap_or_else(|| (String::new(), DEFAULT_PORT));
        let ts = now_ms();
        conn.execute(
            "INSERT INTO browser_profile (id, name, platforms, user_data_dir, debug_port, session_json, created_at, updated_at)
             VALUES (?1, ?2, 'both', ?3, ?4, '{}', ?5, ?5)",
            params![DEFAULT_PROFILE_ID, "默认", udir, port as i64, ts],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn profiles_root(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?
        .join("browser-profiles");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create browser-profiles: {e}"))?;
    Ok(dir)
}

fn default_profile_dir(app: &AppHandle, profile_id: &str) -> Result<PathBuf, String> {
    let dir = profiles_root(app)?.join(profile_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create profile dir: {e}"))?;
    Ok(dir)
}

fn path_allowed(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let full = path.to_string_lossy().to_ascii_lowercase();
    let hints = [
        "google chrome",
        "chrome",
        "chromium",
        "msedge",
        "microsoft edge",
    ];
    hints.iter().any(|h| name.contains(h) || full.contains(h))
}

fn kind_from_path(path: &str) -> &str {
    let lower = path.to_ascii_lowercase();
    if lower.contains("edge") || lower.contains("msedge") {
        "edge"
    } else if lower.contains("chromium") {
        "chromium"
    } else {
        "chrome"
    }
}

fn candidate(kind: &str, label: &str, path: PathBuf) -> Option<BrowserCandidate> {
    if path.is_file() || path.exists() {
        Some(BrowserCandidate {
            kind: kind.to_string(),
            label: label.to_string(),
            path: path.to_string_lossy().into_owned(),
        })
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn macos_candidates() -> Vec<BrowserCandidate> {
    let mut out = Vec::new();
    let paths = [
        (
            "chrome",
            "Google Chrome",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        ),
        (
            "edge",
            "Microsoft Edge",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ),
        (
            "chromium",
            "Chromium",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ),
    ];
    for (kind, label, p) in paths {
        if let Some(c) = candidate(kind, label, PathBuf::from(p)) {
            out.push(c);
        }
    }
    out
}

#[cfg(target_os = "windows")]
fn windows_candidates() -> Vec<BrowserCandidate> {
    let mut out = Vec::new();
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let pf = std::env::var_os("PROGRAMFILES").map(PathBuf::from);
    let pf86 = std::env::var_os("PROGRAMFILES(X86)").map(PathBuf::from);
    let mut paths: Vec<(&str, &str, PathBuf)> = Vec::new();
    if let Some(ref p) = local {
        paths.push((
            "chrome",
            "Google Chrome",
            p.join("Google/Chrome/Application/chrome.exe"),
        ));
        paths.push((
            "edge",
            "Microsoft Edge",
            p.join("Microsoft/Edge/Application/msedge.exe"),
        ));
    }
    for root in [pf, pf86].into_iter().flatten() {
        paths.push((
            "chrome",
            "Google Chrome",
            root.join("Google/Chrome/Application/chrome.exe"),
        ));
        paths.push((
            "edge",
            "Microsoft Edge",
            root.join("Microsoft/Edge/Application/msedge.exe"),
        ));
    }
    for (kind, label, path) in paths {
        if let Some(c) = candidate(kind, label, path) {
            if !out.iter().any(|x: &BrowserCandidate| x.path == c.path) {
                out.push(c);
            }
        }
    }
    out
}

fn detect_first_exe() -> (String, String) {
    #[cfg(target_os = "macos")]
    {
        if let Some(c) = macos_candidates().into_iter().next() {
            return (c.path, c.kind);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(c) = windows_candidates().into_iter().next() {
            return (c.path, c.kind);
        }
    }
    (String::new(), "chrome".into())
}

fn load_global(conn: &Connection) -> Result<(String, String, bool), String> {
    let row = conn
        .query_row(
            "SELECT browser_executable, browser_kind, auto_launch
             FROM browser_automation WHERE id = ?1",
            params![GLOBAL_ID],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? != 0,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(r) = row {
        return Ok(r);
    }
    let (exe, kind) = detect_first_exe();
    Ok((exe, kind, true))
}

fn save_global(
    conn: &Connection,
    exe: &str,
    kind: &str,
    auto_launch: bool,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO browser_automation (
            id, browser_executable, browser_kind, user_data_dir, debug_port, auto_launch, updated_at
         ) VALUES (?1, ?2, ?3, '', 9222, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
            browser_executable=excluded.browser_executable,
            browser_kind=excluded.browser_kind,
            auto_launch=excluded.auto_launch,
            updated_at=excluded.updated_at",
        params![
            GLOBAL_ID,
            exe,
            kind,
            if auto_launch { 1 } else { 0 },
            now_ms(),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn load_profile(conn: &Connection, id: &str) -> Result<Option<BrowserProfile>, String> {
    conn.query_row(
        "SELECT id, name, platforms, user_data_dir, debug_port, session_json
         FROM browser_profile WHERE id = ?1",
        params![id],
        |row| {
            Ok(BrowserProfile {
                id: row.get(0)?,
                name: row.get(1)?,
                platforms: row.get(2)?,
                user_data_dir: row.get(3)?,
                debug_port: row.get::<_, i64>(4)? as u16,
                session_json: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn list_profiles_db(conn: &Connection) -> Result<Vec<BrowserProfile>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, platforms, user_data_dir, debug_port, session_json
             FROM browser_profile ORDER BY created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(BrowserProfile {
                id: row.get(0)?,
                name: row.get(1)?,
                platforms: row.get(2)?,
                user_data_dir: row.get(3)?,
                debug_port: row.get::<_, i64>(4)? as u16,
                session_json: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn used_ports(conn: &Connection, except_id: Option<&str>) -> Result<Vec<u16>, String> {
    let profiles = list_profiles_db(conn)?;
    Ok(profiles
        .into_iter()
        .filter(|p| except_id.map(|id| p.id != id).unwrap_or(true))
        .map(|p| p.debug_port)
        .collect())
}

fn next_free_port(used: &[u16]) -> u16 {
    let mut port = DEFAULT_PORT;
    while used.contains(&port) || port_open(port) {
        port = port.saturating_add(1);
        if port < DEFAULT_PORT {
            break;
        }
    }
    port
}

fn port_open(port: u16) -> bool {
    let addr = format!("127.0.0.1:{port}");
    let Ok(sock) = addr.parse() else {
        return false;
    };
    TcpStream::connect_timeout(&sock, Duration::from_millis(300)).is_ok()
}

fn fetch_cdp_version(port: u16) -> Option<(String, String)> {
    let url = format!("http://127.0.0.1:{port}/json/version");
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout_read(Duration::from_secs(3))
        .build();
    let resp = agent.get(&url).call().ok()?;
    let body: serde_json::Value = resp.into_json().ok()?;
    let version = body
        .get("Browser")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let ws = body
        .get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if version.is_empty() && ws.is_empty() {
        None
    } else {
        Some((version, ws))
    }
}

fn url_host_lower(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn url_path_lower(url: &str) -> String {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let path = after_scheme.find('/').map(|i| &after_scheme[i..]).unwrap_or("/");
    path.split('?')
        .next()
        .unwrap_or(path)
        .split('#')
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
}

/// Whether an existing tab URL already covers the destination (no new tab needed).
fn tab_satisfies_url(current: &str, target: &str) -> bool {
    let ch = url_host_lower(current);
    let th = url_host_lower(target);
    if ch.is_empty() || th.is_empty() || ch != th {
        return false;
    }
    let cp = url_path_lower(current);
    let tp = url_path_lower(target);

    // 创作服务平台：同 host 即复用；仅当目标明确要发布页时要求已在 /publish
    if th.contains("creator.xiaohongshu.com") || th.contains("creator.douyin.com") {
        if tp.contains("/publish") || tp.contains("/content/upload") || tp.contains("/upload") {
            return cp.contains("/publish")
                || cp.contains("/content/upload")
                || cp.contains("/upload");
        }
        // 登录页：已在创作者域任意页（含已登录首页）即可
        if tp.contains("/login") {
            return true;
        }
        return true;
    }

    // 其它站点：路径相同或当前已在目标路径下
    cp == tp || cp.starts_with(&tp) || tp.starts_with(&cp)
}

/// Prefer an existing page tab on the same host (creator platform / same site).
fn pick_reuse_tab<'a>(targets: &'a [CdpTarget], target_url: &str) -> Option<&'a CdpTarget> {
    let th = url_host_lower(target_url);
    if th.is_empty() {
        return None;
    }
    let tp = url_path_lower(target_url);
    let pages: Vec<&CdpTarget> = targets
        .iter()
        .filter(|t| t.tab_type == "page" && !t.id.is_empty())
        .filter(|t| url_host_lower(&t.url) == th)
        .collect();
    if pages.is_empty() {
        return None;
    }
    // Prefer path match (e.g. already on /publish)
    if let Some(hit) = pages.iter().find(|t| {
        let cp = url_path_lower(&t.url);
        if tp.contains("/publish") || tp.contains("/upload") {
            return cp.contains("/publish") || cp.contains("/upload") || cp.contains("/content/upload");
        }
        cp == tp || cp.starts_with(&tp)
    }) {
        return Some(*hit);
    }
    pages.first().copied()
}

fn activate_cdp_target(port: u16, target_id: &str) -> Result<(), String> {
    let api = format!("http://127.0.0.1:{port}/json/activate/{target_id}");
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout_read(Duration::from_secs(5))
        .build();
    agent
        .get(&api)
        .call()
        .map(|_| ())
        .map_err(|e| format!("激活标签失败: {e}"))
}

fn open_url_via_cdp(port: u16, url: &str) -> Result<(), String> {
    // Reuse existing tab for the same creator/site host — never open duplicates.
    if let Ok(targets) = fetch_cdp_targets_raw(port) {
        if let Some(tab) = pick_reuse_tab(&targets, url) {
            let _ = activate_cdp_target(port, &tab.id);
            if !tab_satisfies_url(&tab.url, url) {
                if !tab.web_socket_debugger_url.is_empty() {
                    let expr = format!(
                        "window.location.assign({})",
                        serde_json::to_string(url).unwrap_or_else(|_| format!("\"{url}\""))
                    );
                    let _ = cdp_runtime_evaluate(&tab.web_socket_debugger_url, &expr, 20);
                }
            }
            return Ok(());
        }
    }

    let encoded = urlencoding_minimal(url);
    let api = format!("http://127.0.0.1:{port}/json/new?{encoded}");
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout_read(Duration::from_secs(5))
        .build();
    match agent.put(&api).call() {
        Ok(_) => Ok(()),
        Err(_) => {
            // Some Chrome builds accept GET
            agent
                .get(&api)
                .call()
                .map(|_| ())
                .map_err(|e| format!("打开页面失败: {e}"))
        }
    }
}

fn urlencoding_minimal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn login_url(platform: &str) -> Result<&'static str, String> {
    match platform.trim().to_ascii_lowercase().as_str() {
        "xhs" | "xiaohongshu" | "小红书" => Ok("https://creator.xiaohongshu.com/login"),
        "douyin" | "dy" | "抖音" => Ok("https://creator.douyin.com/"),
        other => Err(format!("未知平台: {other}（支持 xhs / douyin）")),
    }
}

fn child_running(child: &mut Child) -> bool {
    match child.try_wait() {
        Ok(None) => true,
        Ok(Some(_)) => false,
        Err(_) => false,
    }
}

fn prune_dead(map: &mut HashMap<String, Child>) {
    let dead: Vec<String> = map
        .iter_mut()
        .filter_map(|(id, child)| {
            if child_running(child) {
                None
            } else {
                Some(id.clone())
            }
        })
        .collect();
    for id in dead {
        map.remove(&id);
    }
}

fn build_status(
    profile: &BrowserProfile,
    exe: &str,
    kind: &str,
    auto_launch: bool,
    running: bool,
) -> BrowserStatus {
    let cdp = fetch_cdp_version(profile.debug_port);
    let cdp_online = cdp.is_some();
    let (browser_version, ws) = match cdp {
        Some((v, w)) => (Some(v), Some(w)),
        None => (None, None),
    };
    let configured = !exe.trim().is_empty();
    let message = if !configured {
        "未配置浏览器路径".into()
    } else if cdp_online {
        "CDP 在线".into()
    } else if running {
        "进程已启动，等待 CDP…".into()
    } else if port_open(profile.debug_port) {
        "端口已占用但无法读取 CDP 版本".into()
    } else {
        "CDP 离线".into()
    };
    BrowserStatus {
        profile_id: profile.id.clone(),
        profile_name: profile.name.clone(),
        configured,
        running,
        cdp_online,
        debug_port: profile.debug_port,
        browser_executable: exe.to_string(),
        user_data_dir: profile.user_data_dir.clone(),
        browser_kind: kind.to_string(),
        auto_launch,
        platforms: profile.platforms.clone(),
        session_json: profile.session_json.clone(),
        message,
        browser_version,
        web_socket_debugger_url: ws,
    }
}

fn ensure_profile_dirs(app: &AppHandle, profile: &mut BrowserProfile) -> Result<(), String> {
    if profile.user_data_dir.trim().is_empty() {
        profile.user_data_dir = default_profile_dir(app, &profile.id)?
            .to_string_lossy()
            .into_owned();
    }
    std::fs::create_dir_all(&profile.user_data_dir)
        .map_err(|e| format!("无法创建用户数据目录: {e}"))?;
    Ok(())
}

fn launch_profile_process(
    exe: &Path,
    profile: &BrowserProfile,
    start_url: Option<&str>,
) -> Result<Child, String> {
    let mut cmd = Command::new(exe);
    cmd.arg(format!("--remote-debugging-port={}", profile.debug_port))
        .arg(format!("--user-data-dir={}", profile.user_data_dir))
        .arg("--remote-allow-origins=*")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-background-networking")
        .arg("--disable-features=TranslateUI")
        .arg(start_url.unwrap_or("about:blank"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn()
        .map_err(|e| format!("启动浏览器失败: {e}"))
}

fn wait_cdp(port: u16, secs: u64) -> bool {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(secs) {
        if fetch_cdp_version(port).is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

fn resolve_profile_id(raw: Option<String>) -> String {
    let id = raw.unwrap_or_default();
    if id.trim().is_empty() {
        DEFAULT_PROFILE_ID.to_string()
    } else {
        id
    }
}

#[tauri::command]
pub fn detect_browsers() -> Result<Vec<BrowserCandidate>, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(macos_candidates())
    }
    #[cfg(target_os = "windows")]
    {
        Ok(windows_candidates())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(Vec::new())
    }
}

#[tauri::command]
pub fn get_browser_config(
    app: AppHandle,
    db: State<'_, crate::library::LibraryState>,
) -> Result<BrowserConfig, String> {
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    let (exe, kind, auto) = load_global(&conn)?;
    let mut profile = load_profile(&conn, DEFAULT_PROFILE_ID)?.unwrap_or(BrowserProfile {
        id: DEFAULT_PROFILE_ID.into(),
        name: "默认".into(),
        platforms: "both".into(),
        user_data_dir: String::new(),
        debug_port: DEFAULT_PORT,
        session_json: "{}".into(),
    });
    drop(conn);
    if profile.user_data_dir.trim().is_empty() {
        profile.user_data_dir = default_profile_dir(&app, &profile.id)?
            .to_string_lossy()
            .into_owned();
    }
    Ok(BrowserConfig {
        browser_executable: exe,
        browser_kind: kind,
        user_data_dir: profile.user_data_dir,
        debug_port: profile.debug_port,
        auto_launch: auto,
    })
}

#[tauri::command]
pub fn save_browser_config(
    app: AppHandle,
    db: State<'_, crate::library::LibraryState>,
    body: BrowserConfig,
) -> Result<BrowserConfig, String> {
    let exe = body.browser_executable.trim().to_string();
    if !exe.is_empty() {
        let path = PathBuf::from(&exe);
        if !path.exists() {
            return Err("浏览器可执行文件不存在".into());
        }
        if !path_allowed(&path) {
            return Err("仅支持 Chrome / Edge / Chromium 可执行文件".into());
        }
    }
    let kind = if body.browser_kind.trim().is_empty() {
        kind_from_path(&exe).to_string()
    } else {
        body.browser_kind.trim().to_string()
    };

    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    save_global(&conn, &exe, &kind, body.auto_launch)?;

    let mut profile = load_profile(&conn, DEFAULT_PROFILE_ID)?.unwrap_or(BrowserProfile {
        id: DEFAULT_PROFILE_ID.into(),
        name: "默认".into(),
        platforms: "both".into(),
        user_data_dir: String::new(),
        debug_port: DEFAULT_PORT,
        session_json: "{}".into(),
    });
    let mut user_data = body.user_data_dir.trim().to_string();
    if user_data.is_empty() {
        user_data = default_profile_dir(&app, DEFAULT_PROFILE_ID)?
            .to_string_lossy()
            .into_owned();
    }
    std::fs::create_dir_all(&user_data).map_err(|e| format!("无法创建用户数据目录: {e}"))?;
    let port = if body.debug_port == 0 {
        DEFAULT_PORT
    } else {
        body.debug_port
    };
    profile.user_data_dir = user_data;
    profile.debug_port = port;
    conn.execute(
        "INSERT INTO browser_profile (id, name, platforms, user_data_dir, debug_port, session_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT(id) DO UPDATE SET
            user_data_dir=excluded.user_data_dir,
            debug_port=excluded.debug_port,
            updated_at=excluded.updated_at",
        params![
            profile.id,
            profile.name,
            profile.platforms,
            profile.user_data_dir,
            profile.debug_port as i64,
            profile.session_json,
            now_ms(),
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(BrowserConfig {
        browser_executable: exe,
        browser_kind: kind,
        user_data_dir: profile.user_data_dir,
        debug_port: profile.debug_port,
        auto_launch: body.auto_launch,
    })
}

#[tauri::command]
pub fn list_browser_profiles(db: State<'_, crate::library::LibraryState>) -> Result<Vec<BrowserProfile>, String> {
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    list_profiles_db(&conn)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertProfileBody {
    pub id: Option<String>,
    pub name: String,
    pub platforms: Option<String>,
    pub user_data_dir: Option<String>,
    pub debug_port: Option<u16>,
}

#[tauri::command]
pub fn upsert_browser_profile(
    app: AppHandle,
    db: State<'_, crate::library::LibraryState>,
    body: UpsertProfileBody,
) -> Result<BrowserProfile, String> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err("Profile 名称不能为空".into());
    }
    let platforms = body
        .platforms
        .unwrap_or_else(|| "both".into())
        .trim()
        .to_ascii_lowercase();
    if !matches!(platforms.as_str(), "both" | "xhs" | "douyin") {
        return Err("platforms 须为 both / xhs / douyin".into());
    }

    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    let id = body
        .id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("p_{}", now_ms()));

    let existing = load_profile(&conn, &id)?;
    let used = used_ports(&conn, Some(&id))?;
    let port = body
        .debug_port
        .filter(|&p| p > 0)
        .unwrap_or_else(|| {
            existing
                .as_ref()
                .map(|p| p.debug_port)
                .unwrap_or_else(|| next_free_port(&used))
        });
    if used.contains(&port) {
        return Err(format!("调试端口 {port} 已被其他 Profile 占用"));
    }

    let mut user_data = body
        .user_data_dir
        .unwrap_or_default()
        .trim()
        .to_string();
    if user_data.is_empty() {
        user_data = existing
            .as_ref()
            .map(|p| p.user_data_dir.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                default_profile_dir(&app, &id)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
    }
    std::fs::create_dir_all(&user_data).map_err(|e| format!("无法创建用户数据目录: {e}"))?;

    let session_json = existing
        .as_ref()
        .map(|p| p.session_json.clone())
        .unwrap_or_else(|| "{}".into());
    let ts = now_ms();
    let created = existing
        .as_ref()
        .map(|_| ts) // unused; keep created via INSERT OR UPDATE
        .unwrap_or(ts);

    if existing.is_some() {
        conn.execute(
            "UPDATE browser_profile SET name=?2, platforms=?3, user_data_dir=?4, debug_port=?5, updated_at=?6
             WHERE id=?1",
            params![id, name, platforms, user_data, port as i64, ts],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "INSERT INTO browser_profile (id, name, platforms, user_data_dir, debug_port, session_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![id, name, platforms, user_data, port as i64, session_json, created],
        )
        .map_err(|e| e.to_string())?;
    }

    load_profile(&conn, &id)?.ok_or_else(|| "保存后读取失败".into())
}

#[tauri::command]
pub fn delete_browser_profile(
    db: State<'_, crate::library::LibraryState>,
    browser: State<'_, BrowserProcess>,
    profile_id: String,
) -> Result<(), String> {
    if profile_id == DEFAULT_PROFILE_ID {
        return Err("不能删除默认 Profile".into());
    }
    {
        let mut guard = browser.0.lock().map_err(|_| "browser lock".to_string())?;
        if let Some(mut child) = guard.remove(&profile_id) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    conn.execute(
        "DELETE FROM browser_profile WHERE id = ?1",
        params![profile_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSessionBody {
    pub profile_id: String,
    pub session_json: String,
}

#[tauri::command]
pub fn update_browser_profile_session(
    db: State<'_, crate::library::LibraryState>,
    body: UpdateSessionBody,
) -> Result<BrowserProfile, String> {
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    // validate json
    let _: serde_json::Value =
        serde_json::from_str(&body.session_json).map_err(|e| format!("sessionJson 无效: {e}"))?;
    conn.execute(
        "UPDATE browser_profile SET session_json=?2, updated_at=?3 WHERE id=?1",
        params![body.profile_id, body.session_json, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    load_profile(&conn, &body.profile_id)?.ok_or_else(|| "Profile 不存在".into())
}

#[tauri::command]
pub fn browser_cdp_status(
    app: AppHandle,
    db: State<'_, crate::library::LibraryState>,
    browser: State<'_, BrowserProcess>,
    profile_id: Option<String>,
) -> Result<BrowserStatus, String> {
    let id = resolve_profile_id(profile_id);
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    let (exe, kind, auto) = load_global(&conn)?;
    let mut profile = load_profile(&conn, &id)?.ok_or_else(|| format!("Profile 不存在: {id}"))?;
    drop(conn);
    ensure_profile_dirs(&app, &mut profile)?;
    let mut guard = browser.0.lock().map_err(|_| "browser lock".to_string())?;
    prune_dead(&mut guard);
    let running = guard.contains_key(&id);
    Ok(build_status(&profile, &exe, &kind, auto, running))
}

#[tauri::command]
pub fn browser_cdp_status_all(
    app: AppHandle,
    db: State<'_, crate::library::LibraryState>,
    browser: State<'_, BrowserProcess>,
) -> Result<BrowserStatusSummary, String> {
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    let (exe, kind, auto) = load_global(&conn)?;
    let mut profiles = list_profiles_db(&conn)?;
    drop(conn);

    let mut guard = browser.0.lock().map_err(|_| "browser lock".to_string())?;
    prune_dead(&mut guard);

    let mut statuses = Vec::new();
    for mut p in profiles.drain(..) {
        let _ = ensure_profile_dirs(&app, &mut p);
        let running = guard.contains_key(&p.id);
        statuses.push(build_status(&p, &exe, &kind, auto, running));
    }

    let online = statuses.iter().filter(|s| s.cdp_online).count();
    let total = statuses.len();
    let primary = statuses
        .iter()
        .find(|s| s.cdp_online)
        .or_else(|| statuses.first());

    let message = if exe.trim().is_empty() {
        "未配置浏览器路径".into()
    } else if total == 0 {
        "暂无 Profile".into()
    } else {
        format!("在线 {online}/{total}")
    };

    Ok(BrowserStatusSummary {
        configured: !exe.trim().is_empty(),
        online_count: online,
        total_count: total,
        message,
        running: primary.map(|s| s.running).unwrap_or(false),
        cdp_online: online > 0,
        debug_port: primary.map(|s| s.debug_port).unwrap_or(DEFAULT_PORT),
        browser_executable: exe,
        user_data_dir: primary
            .map(|s| s.user_data_dir.clone())
            .unwrap_or_default(),
        browser_kind: kind,
        auto_launch: auto,
        browser_version: primary.and_then(|s| s.browser_version.clone()),
        web_socket_debugger_url: primary.and_then(|s| s.web_socket_debugger_url.clone()),
        profiles: statuses,
    })
}

#[tauri::command]
pub fn launch_browser_cdp(
    app: AppHandle,
    db: State<'_, crate::library::LibraryState>,
    browser: State<'_, BrowserProcess>,
    profile_id: Option<String>,
) -> Result<BrowserStatus, String> {
    let id = resolve_profile_id(profile_id);
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    let (exe, kind, auto) = load_global(&conn)?;
    let mut profile = load_profile(&conn, &id)?.ok_or_else(|| format!("Profile 不存在: {id}"))?;
    drop(conn);

    if exe.trim().is_empty() {
        return Err("请先配置浏览器可执行文件路径".into());
    }
    let exe_path = PathBuf::from(&exe);
    if !exe_path.exists() {
        return Err("浏览器可执行文件不存在".into());
    }
    if !path_allowed(&exe_path) {
        return Err("仅支持 Chrome / Edge / Chromium".into());
    }
    ensure_profile_dirs(&app, &mut profile)?;

    {
        let mut guard = browser.0.lock().map_err(|_| "browser lock".to_string())?;
        prune_dead(&mut guard);
        if !guard.contains_key(&id) {
            if fetch_cdp_version(profile.debug_port).is_none() {
                let child = launch_profile_process(&exe_path, &profile, None)?;
                guard.insert(id.clone(), child);
            }
        }
    }

    wait_cdp(profile.debug_port, 15);
    let mut guard = browser.0.lock().map_err(|_| "browser lock".to_string())?;
    prune_dead(&mut guard);
    let running = guard.contains_key(&id);
    Ok(build_status(&profile, &exe, &kind, auto, running))
}

#[tauri::command]
pub fn stop_browser_cdp(
    app: AppHandle,
    db: State<'_, crate::library::LibraryState>,
    browser: State<'_, BrowserProcess>,
    profile_id: Option<String>,
) -> Result<BrowserStatus, String> {
    let id = resolve_profile_id(profile_id);
    {
        let mut guard = browser.0.lock().map_err(|_| "browser lock".to_string())?;
        if let Some(mut child) = guard.remove(&id) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    let (exe, kind, auto) = load_global(&conn)?;
    let mut profile = load_profile(&conn, &id)?.ok_or_else(|| format!("Profile 不存在: {id}"))?;
    drop(conn);
    let _ = ensure_profile_dirs(&app, &mut profile);
    Ok(build_status(&profile, &exe, &kind, auto, false))
}

#[tauri::command]
pub fn login_browser_platform(
    app: AppHandle,
    db: State<'_, crate::library::LibraryState>,
    browser: State<'_, BrowserProcess>,
    profile_id: Option<String>,
    platform: String,
) -> Result<BrowserStatus, String> {
    let id = resolve_profile_id(profile_id);
    let url = login_url(&platform)?;

    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    let (exe, kind, auto) = load_global(&conn)?;
    let mut profile = load_profile(&conn, &id)?.ok_or_else(|| {
        // Auto-create default if missing
        format!("Profile 不存在: {id}")
    })?;
    drop(conn);

    if exe.trim().is_empty() {
        return Err("请先配置浏览器可执行文件路径".into());
    }
    let exe_path = PathBuf::from(&exe);
    if !exe_path.exists() {
        return Err("浏览器可执行文件不存在".into());
    }
    ensure_profile_dirs(&app, &mut profile)?;

    let already_online = fetch_cdp_version(profile.debug_port).is_some();
    {
        let mut guard = browser.0.lock().map_err(|_| "browser lock".to_string())?;
        prune_dead(&mut guard);
        if !already_online && !guard.contains_key(&id) {
            let child = launch_profile_process(&exe_path, &profile, Some(url))?;
            guard.insert(id.clone(), child);
        }
    }

    if !wait_cdp(profile.debug_port, 15) {
        return Err("浏览器已启动但 CDP 未就绪".into());
    }

    if already_online {
        open_url_via_cdp(profile.debug_port, url)?;
    }

    let mut guard = browser.0.lock().map_err(|_| "browser lock".to_string())?;
    prune_dead(&mut guard);
    let running = guard.contains_key(&id) || fetch_cdp_version(profile.debug_port).is_some();
    Ok(build_status(&profile, &exe, &kind, auto, running))
}


#[tauri::command]
pub fn open_browser_url(
    app: AppHandle,
    db: State<'_, crate::library::LibraryState>,
    browser: State<'_, BrowserProcess>,
    profile_id: Option<String>,
    url: String,
) -> Result<BrowserStatus, String> {
    let id = resolve_profile_id(profile_id);
    let target = url.trim();
    if target.is_empty() {
        return Err("URL 不能为空".into());
    }
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    let (exe, kind, auto) = load_global(&conn)?;
    let mut profile = load_profile(&conn, &id)?.ok_or_else(|| format!("Profile 不存在: {id}"))?;
    drop(conn);

    if exe.trim().is_empty() {
        return Err("请先配置浏览器可执行文件路径".into());
    }
    let exe_path = PathBuf::from(&exe);
    if !exe_path.exists() {
        return Err("浏览器可执行文件不存在".into());
    }
    ensure_profile_dirs(&app, &mut profile)?;

    let already_online = fetch_cdp_version(profile.debug_port).is_some();
    {
        let mut guard = browser.0.lock().map_err(|_| "browser lock".to_string())?;
        prune_dead(&mut guard);
        if !already_online && !guard.contains_key(&id) {
            let child = launch_profile_process(&exe_path, &profile, Some(target))?;
            guard.insert(id.clone(), child);
        }
    }
    if !wait_cdp(profile.debug_port, 15) {
        return Err("浏览器已启动但 CDP 未就绪".into());
    }
    if already_online {
        open_url_via_cdp(profile.debug_port, target)?;
    }
    let mut guard = browser.0.lock().map_err(|_| "browser lock".to_string())?;
    prune_dead(&mut guard);
    let running = guard.contains_key(&id) || fetch_cdp_version(profile.debug_port).is_some();
    Ok(build_status(&profile, &exe, &kind, auto, running))
}


#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTabInfo {
    pub id: String,
    pub title: String,
    pub url: String,
    #[serde(rename = "type")]
    pub tab_type: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTabsSnapshot {
    pub profile_id: String,
    pub profile_name: String,
    pub debug_port: u16,
    pub cdp_online: bool,
    pub browser_version: Option<String>,
    pub tab_count: usize,
    pub page_count: usize,
    pub tabs: Vec<BrowserTabInfo>,
    pub message: String,
}

#[derive(Debug, Clone)]
struct CdpTarget {
    id: String,
    title: String,
    url: String,
    tab_type: String,
    web_socket_debugger_url: String,
}

fn fetch_cdp_targets_raw(port: u16) -> Result<Vec<CdpTarget>, String> {
    let url = format!("http://127.0.0.1:{port}/json/list");
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout_read(Duration::from_secs(5))
        .build();
    let resp = match agent.get(&url).call() {
        Ok(r) => r,
        Err(_) => {
            let fallback = format!("http://127.0.0.1:{port}/json");
            agent
                .get(&fallback)
                .call()
                .map_err(|e| format!("读取标签列表失败: {e}"))?
        }
    };
    let body: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析标签列表失败: {e}"))?;
    let arr = body
        .as_array()
        .ok_or_else(|| "标签列表格式无效".to_string())?;
    let mut tabs = Vec::with_capacity(arr.len());
    for item in arr {
        let tab_type = item
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        tabs.push(CdpTarget {
            id: item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            title: item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("(无标题)")
                .to_string(),
            url: item
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            tab_type,
            web_socket_debugger_url: item
                .get("webSocketDebuggerUrl")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        });
    }
    Ok(tabs)
}

fn fetch_cdp_targets(port: u16) -> Result<Vec<BrowserTabInfo>, String> {
    Ok(fetch_cdp_targets_raw(port)?
        .into_iter()
        .map(|t| BrowserTabInfo {
            id: t.id,
            title: t.title,
            url: t.url,
            tab_type: t.tab_type,
        })
        .collect())
}

/// Pick a page target for XHS DOM probe (prefer xiaohongshu.com).
fn pick_xhs_page_target(targets: &[CdpTarget], tab_id: Option<&str>) -> Result<CdpTarget, String> {
    if let Some(id) = tab_id.map(str::trim).filter(|s| !s.is_empty()) {
        return targets
            .iter()
            .find(|t| t.id == id)
            .cloned()
            .ok_or_else(|| format!("未找到标签 id={id}"));
    }
    let pages: Vec<&CdpTarget> = targets.iter().filter(|t| t.tab_type == "page").collect();
    if pages.is_empty() {
        return Err("当前没有 page 类型标签".into());
    }
    let preferred = pages.iter().find(|t| {
        let u = t.url.to_ascii_lowercase();
        u.contains("xiaohongshu.com") && !u.contains("creator.xiaohongshu.com")
    });
    let chosen = preferred
        .or_else(|| {
            pages
                .iter()
                .find(|t| t.url.to_ascii_lowercase().contains("xiaohongshu.com"))
        })
        .copied()
        .unwrap_or(pages[0]);
    if chosen.web_socket_debugger_url.is_empty() {
        return Err("目标标签缺少 webSocketDebuggerUrl".into());
    }
    Ok(chosen.clone())
}

fn parse_ws_host_port(ws_url: &str) -> Result<(String, u16), String> {
    let rest = ws_url
        .strip_prefix("ws://")
        .or_else(|| ws_url.strip_prefix("wss://"))
        .ok_or_else(|| "CDP WebSocket 仅支持 ws:// 或 wss://".to_string())?;
    let authority = rest.split('/').next().unwrap_or("").trim();
    if authority.is_empty() {
        return Err("无效 WebSocket URL".into());
    }
    if let Some((host, port_s)) = authority.rsplit_once(':') {
        let port: u16 = port_s
            .parse()
            .map_err(|_| format!("无效端口: {port_s}"))?;
        Ok((host.to_string(), port))
    } else {
        let default_port = if ws_url.starts_with("wss://") {
            443
        } else {
            80
        };
        Ok((authority.to_string(), default_port))
    }
}

fn cdp_runtime_evaluate(
    ws_url: &str,
    expression: &str,
    timeout_secs: u64,
) -> Result<serde_json::Value, String> {
    use std::net::TcpStream as StdTcp;
    use tungstenite::{client, protocol::Message};

    let timeout = timeout_secs.max(3);
    let (host, port) = parse_ws_host_port(ws_url)?;
    let addr = format!("{host}:{port}");
    let stream = StdTcp::connect_timeout(
        &addr
            .parse()
            .map_err(|e| format!("解析 CDP 地址失败: {e}"))?,
        Duration::from_secs(3),
    )
    .map_err(|e| format!("连接 CDP WebSocket 失败: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(timeout)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .ok();

    let (mut socket, _) =
        client(ws_url, stream).map_err(|e| format!("CDP WebSocket 握手失败: {e}"))?;

    let req_id = 1u64;
    let payload = serde_json::json!({
        "id": req_id,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": true,
        }
    });
    socket
        .send(Message::Text(payload.to_string()))
        .map_err(|e| format!("发送 Runtime.evaluate 失败: {e}"))?;

    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        if Instant::now() > deadline {
            return Err("等待 Runtime.evaluate 响应超时".into());
        }
        let msg = match socket.read() {
            Ok(m) => m,
            Err(e) => return Err(format!("读取 CDP 响应失败: {e}")),
        };
        let text = match msg {
            Message::Text(t) => t,
            Message::Ping(data) => {
                let _ = socket.send(Message::Pong(data));
                continue;
            }
            Message::Close(_) => return Err("CDP WebSocket 已关闭".into()),
            _ => continue,
        };
        let body: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("解析 CDP 响应失败: {e}"))?;
        if body.get("id").and_then(|v| v.as_u64()) != Some(req_id) {
            continue;
        }
        if let Some(err) = body.get("error") {
            return Err(format!(
                "Runtime.evaluate 错误: {}",
                err.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&err.to_string())
            ));
        }
        let result = body
            .pointer("/result/result")
            .cloned()
            .ok_or_else(|| "Runtime.evaluate 无 result".to_string())?;
        if result.get("subtype").and_then(|v| v.as_str()) == Some("error") {
            let desc = result
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("JS 执行错误");
            return Err(desc.to_string());
        }
        return Ok(result.get("value").cloned().unwrap_or(serde_json::Value::Null));
    }
}

/// Top banner while AI drives the CDP browser (matches Playwright humanize).
const AI_BROWSER_BANNER_SHOW_JS: &str = r#"(function(){
  var ID = 'ss-ai-browser-banner';
  var el = document.getElementById(ID);
  if (!el) {
    el = document.createElement('div');
    el.id = ID;
    el.setAttribute('data-ss-ai-banner', '1');
    el.textContent = 'AI正在操作浏览器';
    el.style.cssText = [
      'position:fixed !important',
      'top:0 !important',
      'left:0 !important',
      'right:0 !important',
      'z-index:2147483647 !important',
      'pointer-events:none !important',
      'padding:10px 16px !important',
      'font:600 14px/1.4 -apple-system,BlinkMacSystemFont,Segoe UI,sans-serif !important',
      'color:#fff !important',
      'background:linear-gradient(90deg,#1a1a1a 0%,#c41e3a 100%) !important',
      'text-align:center !important',
      'letter-spacing:0.02em !important',
      'box-shadow:0 2px 12px rgba(0,0,0,.35) !important',
      'display:block !important'
    ].join(';');
    var root = document.documentElement || document.body;
    if (root) root.appendChild(el);
  } else {
    el.style.setProperty('display', 'block', 'important');
  }
  return { ok:true, visible:true };
})()"#;

const AI_BROWSER_BANNER_CLEAR_JS: &str = r#"(function(){
  var el = document.getElementById('ss-ai-browser-banner');
  if (el) el.remove();
  return { ok:true, visible:false };
})()"#;

fn cdp_set_ai_banner(ws: &str, visible: bool) -> Result<(), String> {
    let js = if visible {
        AI_BROWSER_BANNER_SHOW_JS
    } else {
        AI_BROWSER_BANNER_CLEAR_JS
    };
    cdp_runtime_evaluate(ws, js, 5).map(|_| ())
}

/// Inject/clear banner on all www / explore xhs page tabs (skip creator).
fn cdp_set_ai_banner_on_xhs_pages(port: u16, visible: bool) -> Result<(String, usize), String> {
    let targets = fetch_cdp_targets_raw(port)?;
    let mut hit = 0usize;
    let mut first_id = String::new();
    for t in targets.iter().filter(|t| t.tab_type == "page") {
        let u = t.url.to_ascii_lowercase();
        if !u.contains("xiaohongshu.com") {
            continue;
        }
        if u.contains("creator.xiaohongshu.com") {
            continue;
        }
        if t.web_socket_debugger_url.is_empty() {
            continue;
        }
        if cdp_set_ai_banner(&t.web_socket_debugger_url, visible).is_ok() {
            hit += 1;
            if first_id.is_empty() {
                first_id = t.id.clone();
            }
        }
    }
    if hit == 0 {
        // Fallback: preferred single target (may be www or first page)
        let target = pick_xhs_page_target(&targets, None)?;
        cdp_set_ai_banner(&target.web_socket_debugger_url, visible)?;
        return Ok((target.id, 1));
    }
    Ok((first_id, hit))
}

/// Show/hide「AI正在操作浏览器」banner on the XHS page of a CDP profile.
#[tauri::command]
pub fn set_browser_ai_banner(
    db: State<'_, crate::library::LibraryState>,
    profile_id: Option<String>,
    visible: bool,
) -> Result<XhsImActionResult, String> {
    let probed_at = now_unix_secs();
    let id = resolve_profile_id(profile_id);
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    let profile = load_profile(&conn, &id)?.ok_or_else(|| format!("Profile 不存在: {id}"))?;
    drop(conn);
    if fetch_cdp_version(profile.debug_port).is_none() {
        return Err(format!("CDP 离线（端口 {}）", profile.debug_port));
    }
    let (tab_id, hit) = cdp_set_ai_banner_on_xhs_pages(profile.debug_port, visible)?;
    Ok(XhsImActionResult {
        ok: true,
        reply_text: String::new(),
        message: if visible {
            format!("已显示 AI 操作横幅（{hit} 个标签）")
        } else {
            format!("已清除 AI 操作横幅（{hit} 个标签）")
        },
        peer_name: String::new(),
        index: None,
        tab_id,
        probed_at,
    })
}

/// JS: find span text「消息」, then sibling .badge-container > .count
const XHS_MESSAGE_BADGE_JS: &str = r#"(function(){
  function textOf(el){ if (!el) return ''; return String(el.textContent || el.innerText || '').replace(/\s+/g,' ').trim(); }
  var spans = Array.prototype.slice.call(document.querySelectorAll('span'));
  var msg = null;
  for (var i=0;i<spans.length;i++){
    if (textOf(spans[i]) === '消息') { msg = spans[i]; break; }
  }
  if (!msg) {
    return { found:false, spanFound:false, badgeFound:false, countFound:false, count:null, countText:'', message:'未找到内容为「消息」的 span' };
  }
  var parent = msg.parentElement;
  var badge = null;
  if (parent) {
    var kids = parent.children;
    for (var j=0;j<kids.length;j++){
      var el = kids[j];
      if (el === msg) continue;
      if (el.classList && el.classList.contains('badge-container')) { badge = el; break; }
    }
  }
  if (!badge && msg.previousElementSibling && msg.previousElementSibling.classList
      && msg.previousElementSibling.classList.contains('badge-container')) {
    badge = msg.previousElementSibling;
  }
  if (!badge && msg.nextElementSibling && msg.nextElementSibling.classList
      && msg.nextElementSibling.classList.contains('badge-container')) {
    badge = msg.nextElementSibling;
  }
  if (!badge) {
    return { found:false, spanFound:true, badgeFound:false, countFound:false, count:null, countText:'', message:'找到「消息」但同级无 badge-container' };
  }
  var countEl = badge.querySelector('div.count') || badge.querySelector('.count');
  if (!countEl) {
    return { found:true, spanFound:true, badgeFound:true, countFound:false, count:null, countText:'', message:'badge-container 内无 count（可能无未读）' };
  }
  var raw = textOf(countEl);
  var num = parseInt(raw, 10);
  return {
    found:true, spanFound:true, badgeFound:true, countFound:true,
    count: isNaN(num) ? null : num,
    countText: raw,
    message: 'ok'
  };
})()"#;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsMessageBadgeProbe {
    pub profile_id: String,
    pub profile_name: String,
    pub debug_port: u16,
    pub cdp_online: bool,
    pub tab_id: String,
    pub tab_title: String,
    pub tab_url: String,
    pub span_found: bool,
    pub badge_found: bool,
    pub count_found: bool,
    pub count: Option<i64>,
    pub count_text: String,
    pub message: String,
    pub probed_at: i64,
}

/// List open targets/tabs for a CDP Profile (debug helper).
#[tauri::command]
pub fn list_browser_tabs(
    db: State<'_, crate::library::LibraryState>,
    profile_id: Option<String>,
) -> Result<BrowserTabsSnapshot, String> {
    let id = resolve_profile_id(profile_id);
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    let profile = load_profile(&conn, &id)?.ok_or_else(|| format!("Profile 不存在: {id}"))?;
    drop(conn);

    let version = fetch_cdp_version(profile.debug_port);
    let cdp_online = version.is_some();
    if !cdp_online {
        return Ok(BrowserTabsSnapshot {
            profile_id: profile.id,
            profile_name: profile.name,
            debug_port: profile.debug_port,
            cdp_online: false,
            browser_version: None,
            tab_count: 0,
            page_count: 0,
            tabs: vec![],
            message: format!("CDP 离线（端口 {}）", profile.debug_port),
        });
    }

    let tabs = fetch_cdp_targets(profile.debug_port)?;
    let page_count = tabs.iter().filter(|t| t.tab_type == "page").count();
    let browser_version = version.map(|(v, _)| v);
    Ok(BrowserTabsSnapshot {
        profile_id: profile.id,
        profile_name: profile.name,
        debug_port: profile.debug_port,
        cdp_online: true,
        browser_version,
        tab_count: tabs.len(),
        page_count,
        message: format!("共 {} 个目标，其中 page {} 个", tabs.len(), page_count),
        tabs,
    })
}

fn validate_http_url(url: &str) -> Result<String, String> {
    let raw = url.trim();
    if raw.is_empty() {
        return Err("URL 不能为空".into());
    }
    let lower = raw.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("仅支持 http:// 或 https:// 链接".into());
    }
    Ok(raw.to_string())
}

/// Probe XHS nav「消息」badge count via CDP Runtime.evaluate.
#[tauri::command]
pub fn probe_xhs_message_badge(
    db: State<'_, crate::library::LibraryState>,
    profile_id: Option<String>,
    tab_id: Option<String>,
) -> Result<XhsMessageBadgeProbe, String> {
    let id = resolve_profile_id(profile_id);
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    let profile = load_profile(&conn, &id)?.ok_or_else(|| format!("Profile 不存在: {id}"))?;
    drop(conn);

    let probed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let version = fetch_cdp_version(profile.debug_port);
    if version.is_none() {
        return Ok(XhsMessageBadgeProbe {
            profile_id: profile.id,
            profile_name: profile.name,
            debug_port: profile.debug_port,
            cdp_online: false,
            tab_id: String::new(),
            tab_title: String::new(),
            tab_url: String::new(),
            span_found: false,
            badge_found: false,
            count_found: false,
            count: None,
            count_text: String::new(),
            message: format!("CDP 离线（端口 {}）", profile.debug_port),
            probed_at,
        });
    }

    let targets = fetch_cdp_targets_raw(profile.debug_port)?;
    let target = pick_xhs_page_target(&targets, tab_id.as_deref())?;
    let value = cdp_runtime_evaluate(&target.web_socket_debugger_url, XHS_MESSAGE_BADGE_JS, 8)?;

    let span_found = value
        .get("spanFound")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let badge_found = value
        .get("badgeFound")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let count_found = value
        .get("countFound")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let count = value.get("count").and_then(|v| {
        if v.is_null() {
            None
        } else {
            v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
        }
    });
    let count_text = value
        .get("countText")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let message = value
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("ok")
        .to_string();

    Ok(XhsMessageBadgeProbe {
        profile_id: profile.id,
        profile_name: profile.name,
        debug_port: profile.debug_port,
        cdp_online: true,
        tab_id: target.id,
        tab_title: target.title,
        tab_url: target.url,
        span_found,
        badge_found,
        count_found,
        count,
        count_text,
        message,
        probed_at,
    })
}

/// Click nav「消息」link.
const XHS_CLICK_MESSAGE_NAV_JS: &str = r#"(function(){
  function textOf(el){ if (!el) return ''; return String(el.textContent || el.innerText || '').replace(/\s+/g,' ').trim(); }
  var spans = Array.prototype.slice.call(document.querySelectorAll('span'));
  var msg = null;
  for (var i=0;i<spans.length;i++){
    if (textOf(spans[i]) === '消息') { msg = spans[i]; break; }
  }
  if (!msg) return { ok:false, message:'未找到「消息」span' };
  var clickable = msg.closest('a.link-wrapper') || msg.closest('a') || msg;
  try { clickable.click(); } catch (e) {
    return { ok:false, message:'点击失败: ' + (e && e.message ? e.message : String(e)) };
  }
  return { ok:true, message:'已点击「消息」' };
})()"#;

/// Wait for IM conversation list.
const XHS_WAIT_CONV_LIST_JS: &str = r#"(async function(){
  function sleep(ms){ return new Promise(function(r){ setTimeout(r, ms); }); }
  var deadline = Date.now() + 8000;
  while (Date.now() < deadline) {
    var scroll = document.querySelector('.xhs-im-conv-list__scroll');
    var items = document.querySelectorAll('.xhs-im-conv-list__scroll .xhs-im-conv-item, .xhs-im-conv-item');
    if (scroll && items.length > 0) {
      return { ok:true, itemCount: items.length, message:'会话列表已就绪' };
    }
    await sleep(250);
  }
  var scroll2 = document.querySelector('.xhs-im-conv-list__scroll');
  return {
    ok:false,
    itemCount: 0,
    message: scroll2 ? '会话列表容器存在但无 item' : '等待会话列表超时'
  };
})()"#;

/// List unread convs and click the first with badge > 0.
const XHS_CLICK_FIRST_UNREAD_CONV_JS: &str = r#"(function(){
  function textOf(el){ if (!el) return ''; return String(el.textContent || el.innerText || '').replace(/\s+/g,' ').trim(); }
  function convNick(item){
    if (!item) return '';
    var nameEl = item.querySelector('.xhs-im-conv-item__name')
      || item.querySelector('.xhs-im-conv-item__title')
      || item.querySelector('.xhs-im-conv-item__top')
      || item.querySelector('[class*="nickname"]')
      || item.querySelector('[class*="nick-name"]')
      || item.querySelector('[class*="user-name"]');
    var t = textOf(nameEl);
    if (!t) return '';
    return t.replace(/\s+\d+\+?\s*$/,'').trim().slice(0, 40);
  }
  var root = document.querySelector('.xhs-im-conv-list__scroll') || document;
  var items = Array.prototype.slice.call(root.querySelectorAll('.xhs-im-conv-item'));
  var conversations = [];
  var openedIndex = null;
  for (var i=0;i<items.length;i++){
    var item = items[i];
    var badgeEl = item.querySelector('.xhs-im-conv-item__badge');
    var badgeRaw = badgeEl ? textOf(badgeEl) : '';
    var badgeNum = parseInt(badgeRaw, 10);
    var badge = isNaN(badgeNum) ? 0 : badgeNum;
    var previewEl = item.querySelector('.xhs-im-conv-item__bottom')
      || item.querySelector('.xhs-im-conv-item__content')
      || item;
    var preview = textOf(previewEl).slice(0, 80);
    var name = convNick(item);
    if (badge > 0) {
      conversations.push({ index:i, badge:badge, preview:preview, name:name });
    }
  }
  if (!conversations.length) {
    return { ok:false, openedIndex:null, conversations:[], peerName:'', message:'未找到 badge>0 的会话' };
  }
  openedIndex = conversations[0].index;
  var target = items[openedIndex];
  try { target.click(); } catch (e) {
    return {
      ok:false, openedIndex:openedIndex, conversations:conversations, peerName: conversations[0].name || '',
      message:'点击会话失败: ' + (e && e.message ? e.message : String(e))
    };
  }
  return {
    ok:true, openedIndex:openedIndex, conversations:conversations, peerName: conversations[0].name || '',
    message:'已点击未读会话 index=' + openedIndex
  };
})()"#;

/// Wait for chat message list.
const XHS_WAIT_MSG_LIST_JS: &str = r#"(async function(){
  function sleep(ms){ return new Promise(function(r){ setTimeout(r, ms); }); }
  var deadline = Date.now() + 8000;
  while (Date.now() < deadline) {
    var list = document.querySelector('.xhs-im-view__main .xhs-im-msg-list-wrap .xhs-im-msg-list')
      || document.querySelector('.xhs-im-msg-list');
    if (list) {
      var n = list.querySelectorAll('.chat-item').length;
      return { ok:true, itemCount:n, message:'对话框已就绪' };
    }
    await sleep(250);
  }
  return { ok:false, itemCount:0, message:'等待对话框消息列表超时' };
})()"#;

/// Scrape recent chat messages (recv left / sent right) + peer nickname.
const XHS_SCRAPE_MESSAGES_JS: &str = r#"(function(){
  function textOf(el){ if (!el) return ''; return String(el.textContent || el.innerText || '').replace(/\s+/g,' ').trim(); }
  function isActiveConv(item){
    if (!item) return false;
    var cn = item.className || '';
    if (/\b(active|is-active|selected|current|is-selected)\b/i.test(cn)) return true;
    if (item.getAttribute('aria-selected') === 'true') return true;
    if (item.getAttribute('data-active') === 'true') return true;
    return !!item.querySelector('[class*="active"], [class*="selected"], [class*="current"]');
  }
  function peerNameFromDom(){
    var header = document.querySelector('.xhs-im-chat-header__name')
      || document.querySelector('.xhs-im-header__title')
      || document.querySelector('.xhs-im-view__header [class*="name"]')
      || document.querySelector('.xhs-im-chat-header [class*="name"]')
      || document.querySelector('[class*="im-chat-header"] [class*="name"]')
      || document.querySelector('[class*="ChatHeader"] [class*="name"]');
    var ht = textOf(header);
    if (ht) return ht.replace(/\s+\d+\+?\s*$/,'').trim().slice(0, 40);
    var root = document.querySelector('.xhs-im-conv-list__scroll') || document;
    var items = Array.prototype.slice.call(root.querySelectorAll('.xhs-im-conv-item'));
    for (var k=0;k<items.length;k++){
      if (!isActiveConv(items[k])) continue;
      var nameEl = items[k].querySelector('.xhs-im-conv-item__name')
        || items[k].querySelector('.xhs-im-conv-item__title')
        || items[k].querySelector('.xhs-im-conv-item__top')
        || items[k].querySelector('[class*="nickname"]')
        || items[k].querySelector('[class*="user-name"]');
      var t = textOf(nameEl);
      if (t) return t.replace(/\s+\d+\+?\s*$/,'').trim().slice(0, 40);
    }
    return '';
  }
  var list = document.querySelector('.xhs-im-view__main .xhs-im-msg-list-wrap .xhs-im-msg-list')
    || document.querySelector('.xhs-im-msg-list');
  if (!list) return { ok:false, messages:[], peerName: peerNameFromDom(), message:'未找到消息列表' };
  var items = Array.prototype.slice.call(list.querySelectorAll('.chat-item'));
  var messages = [];
  for (var i=0;i<items.length;i++){
    var item = items[i];
    var direction = 'unknown';
    if (item.querySelector('.chat-item__bubble-row--left')
        || item.querySelector('.chat-item__bubble-row.chat-item__bubble-row--left')) {
      direction = 'recv';
    } else if (item.querySelector('.chat-item__content--right')
        || item.querySelector('.chat-item__content.chat-item__content--right')) {
      direction = 'sent';
    }
    var bubble = item.querySelector('.xhs-im-bubble__text');
    var text = bubble ? textOf(bubble) : textOf(item);
    if (!text) continue;
    messages.push({ direction:direction, text:text });
  }
  if (messages.length > 40) messages = messages.slice(messages.length - 40);
  return {
    ok:true,
    messages:messages,
    peerName: peerNameFromDom(),
    message:'读取 ' + messages.length + ' 条消息'
  };
})()"#;

const XHS_POLITE_REPLIES: &[&str] = &[
    "您好，收到了，稍后回复您～",
    "谢谢您的消息，我这边看到了。",
    "好的，感谢联系，我会尽快处理。",
    "您好呀，已收到，稍等片刻～",
    "收到啦，谢谢您！",
    "好的呢，我记下了，稍后回复您。",
    "感谢您的留言，马上看一下。",
    "您好，消息已收到，请稍候。",
];

fn pick_polite_reply() -> &'static str {
    let n = XHS_POLITE_REPLIES.len().max(1);
    let idx = (now_unix_secs().unsigned_abs() as usize)
        .wrapping_mul(2654435761)
        % n;
    XHS_POLITE_REPLIES[idx]
}

/// Fill IM editor human-like (per-char typing) then Enter to send.
/// `text_json` must be a JSON-encoded string literal.
fn xhs_send_reply_js(text_json: &str) -> String {
    format!(
        r#"(async function(){{
  var text = {text_json};
  function sleep(ms){{ return new Promise(function(r){{ setTimeout(r, ms); }}); }}
  function rand(min, max){{ return min + Math.floor(Math.random() * (max - min + 1)); }}
  function dispatchKey(el, type, key, code, keyCode){{
    var opts = {{
      key: key, code: code, keyCode: keyCode, which: keyCode,
      bubbles: true, cancelable: true, composed: true
    }};
    el.dispatchEvent(new KeyboardEvent(type, opts));
  }}
  function typeChar(el, ch){{
    var isInput = el.tagName === 'TEXTAREA' || el.tagName === 'INPUT';
    var code = ch === ' ' ? 'Space' : ('Key' + ch.toUpperCase());
    var keyCode = ch.length === 1 ? ch.charCodeAt(0) : 0;
    dispatchKey(el, 'keydown', ch, code, keyCode);
    if (isInput) {{
      el.value = (el.value || '') + ch;
      el.dispatchEvent(new Event('input', {{ bubbles:true }}));
    }} else {{
      var ok = false;
      try {{ ok = document.execCommand('insertText', false, ch); }} catch (e) {{ ok = false; }}
      if (!ok) {{
        el.textContent = (el.textContent || '') + ch;
        try {{
          el.dispatchEvent(new InputEvent('input', {{ bubbles:true, data:ch, inputType:'insertText' }}));
        }} catch (e2) {{
          el.dispatchEvent(new Event('input', {{ bubbles:true }}));
        }}
      }} else {{
        try {{
          el.dispatchEvent(new InputEvent('input', {{ bubbles:true, data:ch, inputType:'insertText' }}));
        }} catch (e3) {{}}
      }}
    }}
    dispatchKey(el, 'keypress', ch, code, keyCode);
    dispatchKey(el, 'keyup', ch, code, keyCode);
  }}
  var wrap = document.querySelector('.xhs-im-input-bar-wrap');
  if (!wrap) return {{ ok:false, message:'未找到 .xhs-im-input-bar-wrap' }};
  var editor = wrap.querySelector('.xhs-im-input-bar-editor')
    || wrap.querySelector('[contenteditable="true"]')
    || wrap.querySelector('textarea')
    || wrap.querySelector('[role="textbox"]');
  if (!editor) return {{ ok:false, message:'未找到 .xhs-im-input-bar-editor' }};
  var editable = editor;
  if (!(editable.isContentEditable || editable.getAttribute('contenteditable') === 'true'
      || editable.tagName === 'TEXTAREA' || editable.tagName === 'INPUT')) {{
    var inner = editor.querySelector('[contenteditable="true"], textarea, [role="textbox"]');
    if (inner) editable = inner;
  }}
  try {{
    editable.scrollIntoView({{ block:'nearest', inline:'nearest' }});
  }} catch (e0) {{}}
  await sleep(rand(180, 420));
  try {{ editable.click(); }} catch (e1) {{}}
  try {{ editable.focus(); }} catch (e2) {{}}
  await sleep(rand(120, 280));
  try {{
    var isInput = editable.tagName === 'TEXTAREA' || editable.tagName === 'INPUT';
    if (isInput) {{
      editable.select && editable.select();
      editable.value = '';
      editable.dispatchEvent(new Event('input', {{ bubbles:true }}));
    }} else {{
      try {{ document.execCommand('selectAll', false, null); }} catch (e3) {{}}
      try {{ document.execCommand('delete', false, null); }} catch (e4) {{
        editable.textContent = '';
        editable.dispatchEvent(new Event('input', {{ bubbles:true }}));
      }}
    }}
  }} catch (e5) {{}}
  await sleep(rand(100, 220));
  try {{
    var chars = Array.from(text);
    for (var i = 0; i < chars.length; i++) {{
      typeChar(editable, chars[i]);
      var delay = rand(45, 110);
      if (chars[i] === '，' || chars[i] === '。' || chars[i] === '！' || chars[i] === '、'
          || chars[i] === ',' || chars[i] === '.' || chars[i] === '!' || chars[i] === '？'
          || chars[i] === '?') {{
        delay += rand(120, 260);
      }}
      if (i > 0 && i % rand(4, 7) === 0) delay += rand(80, 200);
      await sleep(delay);
    }}
  }} catch (e) {{
    return {{ ok:false, message:'拟人输入失败: ' + (e && e.message ? e.message : String(e)) }};
  }}
  await sleep(rand(350, 900));
  try {{ editable.focus(); }} catch (e6) {{}}
  await sleep(rand(80, 180));
  // 只派发一次 keydown Enter，避免 keypress/keyup 被页面再次当成发送
  try {{
    dispatchKey(editable, 'keydown', 'Enter', 'Enter', 13);
  }} catch (e) {{
    return {{ ok:false, message:'发送回车失败: ' + (e && e.message ? e.message : String(e)) }};
  }}
  await sleep(rand(200, 400));
  return {{ ok:true, text:text, message:'已拟人发送: ' + text }};
}})()"#
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsUnreadStep {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsUnreadConversation {
    pub index: usize,
    pub badge: i64,
    pub preview: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsChatMessage {
    pub direction: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsUnreadReadResult {
    pub profile_id: String,
    pub profile_name: String,
    pub debug_port: u16,
    pub cdp_online: bool,
    pub tab_id: String,
    pub tab_title: String,
    pub tab_url: String,
    pub nav_badge: Option<i64>,
    pub clicked_nav: bool,
    pub conversations: Vec<XhsUnreadConversation>,
    pub opened_index: Option<usize>,
    pub peer_name: String,
    pub messages: Vec<XhsChatMessage>,
    pub replied: bool,
    pub reply_text: String,
    pub steps: Vec<XhsUnreadStep>,
    pub message: String,
    pub probed_at: i64,
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn push_step(steps: &mut Vec<XhsUnreadStep>, name: &str, ok: bool, detail: impl Into<String>) {
    steps.push(XhsUnreadStep {
        name: name.to_string(),
        ok,
        detail: detail.into(),
    });
}

fn json_i64(v: &serde_json::Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| {
        if x.is_null() {
            None
        } else {
            x.as_i64().or_else(|| x.as_f64().map(|f| f as i64))
        }
    })
}

fn json_bool(v: &serde_json::Value, key: &str) -> bool {
    v.get(key).and_then(|x| x.as_bool()).unwrap_or(false)
}

fn json_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

/// Read first unread IM conversation messages via CDP DOM clicks + scrape.
#[tauri::command]
pub fn read_xhs_unread_messages(
    db: State<'_, crate::library::LibraryState>,
    profile_id: Option<String>,
    tab_id: Option<String>,
) -> Result<XhsUnreadReadResult, String> {
    let id = resolve_profile_id(profile_id);
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    let profile = load_profile(&conn, &id)?.ok_or_else(|| format!("Profile 不存在: {id}"))?;
    drop(conn);

    let probed_at = now_unix_secs();
    let mut steps: Vec<XhsUnreadStep> = Vec::new();

    if fetch_cdp_version(profile.debug_port).is_none() {
        push_step(
            &mut steps,
            "cdp",
            false,
            format!("CDP 离线（端口 {}）", profile.debug_port),
        );
        return Ok(XhsUnreadReadResult {
            profile_id: profile.id,
            profile_name: profile.name,
            debug_port: profile.debug_port,
            cdp_online: false,
            tab_id: String::new(),
            tab_title: String::new(),
            tab_url: String::new(),
            nav_badge: None,
            clicked_nav: false,
            conversations: vec![],
            opened_index: None,
            peer_name: String::new(),
            messages: vec![],
            replied: false,
            reply_text: String::new(),
            steps,
            message: format!("CDP 离线（端口 {}）", profile.debug_port),
            probed_at,
        });
    }

    let targets = fetch_cdp_targets_raw(profile.debug_port)?;
    let target = pick_xhs_page_target(&targets, tab_id.as_deref())?;
    let ws = target.web_socket_debugger_url.clone();

    // 1) probe nav badge
    let badge_val = cdp_runtime_evaluate(&ws, XHS_MESSAGE_BADGE_JS, 8)?;
    let nav_badge = json_i64(&badge_val, "count");
    let badge_msg = json_str(&badge_val, "message");
    let has_unread = nav_badge.map(|n| n > 0).unwrap_or(false);
    push_step(
        &mut steps,
        "probe_badge",
        true,
        format!(
            "角标={:?} · {}",
            nav_badge,
            if badge_msg.is_empty() {
                "ok"
            } else {
                &badge_msg
            }
        ),
    );

    if !has_unread {
        push_step(&mut steps, "skip", true, "角标为空或 <=0，跳过点击");
        return Ok(XhsUnreadReadResult {
            profile_id: profile.id,
            profile_name: profile.name,
            debug_port: profile.debug_port,
            cdp_online: true,
            tab_id: target.id,
            tab_title: target.title,
            tab_url: target.url,
            nav_badge,
            clicked_nav: false,
            conversations: vec![],
            opened_index: None,
            peer_name: String::new(),
            messages: vec![],
            replied: false,
            reply_text: String::new(),
            steps,
            message: "无未读消息（导航角标 <=0）".into(),
            probed_at,
        });
    }

    // 2) click 消息
    let click_nav = cdp_runtime_evaluate(&ws, XHS_CLICK_MESSAGE_NAV_JS, 8)?;
    let clicked_nav = json_bool(&click_nav, "ok");
    push_step(
        &mut steps,
        "click_nav",
        clicked_nav,
        json_str(&click_nav, "message"),
    );
    if !clicked_nav {
        return Ok(XhsUnreadReadResult {
            profile_id: profile.id,
            profile_name: profile.name,
            debug_port: profile.debug_port,
            cdp_online: true,
            tab_id: target.id,
            tab_title: target.title,
            tab_url: target.url,
            nav_badge,
            clicked_nav: false,
            conversations: vec![],
            opened_index: None,
            peer_name: String::new(),
            messages: vec![],
            replied: false,
            reply_text: String::new(),
            steps,
            message: json_str(&click_nav, "message"),
            probed_at,
        });
    }
    thread::sleep(Duration::from_millis(500));

    // 3) wait conv list
    let wait_conv = cdp_runtime_evaluate(&ws, XHS_WAIT_CONV_LIST_JS, 15)?;
    let conv_ready = json_bool(&wait_conv, "ok");
    push_step(
        &mut steps,
        "wait_conv_list",
        conv_ready,
        json_str(&wait_conv, "message"),
    );
    if !conv_ready {
        return Ok(XhsUnreadReadResult {
            profile_id: profile.id,
            profile_name: profile.name,
            debug_port: profile.debug_port,
            cdp_online: true,
            tab_id: target.id,
            tab_title: target.title,
            tab_url: target.url,
            nav_badge,
            clicked_nav: true,
            conversations: vec![],
            opened_index: None,
            peer_name: String::new(),
            messages: vec![],
            replied: false,
            reply_text: String::new(),
            steps,
            message: json_str(&wait_conv, "message"),
            probed_at,
        });
    }
    thread::sleep(Duration::from_millis(400));

    // 4) click first unread conv
    let click_conv = cdp_runtime_evaluate(&ws, XHS_CLICK_FIRST_UNREAD_CONV_JS, 8)?;
    let conv_ok = json_bool(&click_conv, "ok");
    let mut conversations: Vec<XhsUnreadConversation> = Vec::new();
    if let Some(arr) = click_conv.get("conversations").and_then(|v| v.as_array()) {
        for c in arr {
            conversations.push(XhsUnreadConversation {
                index: c
                    .get("index")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize,
                badge: json_i64(c, "badge").unwrap_or(0),
                preview: json_str(c, "preview"),
                name: json_str(c, "name"),
            });
        }
    }
    let opened_index = click_conv
        .get("openedIndex")
        .and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_u64().map(|n| n as usize)
            }
        });
    let mut peer_name = json_str(&click_conv, "peerName");
    if peer_name.is_empty() {
        if let Some(i) = opened_index {
            if let Some(c) = conversations.iter().find(|c| c.index == i) {
                peer_name = c.name.clone();
            }
        }
    }
    push_step(
        &mut steps,
        "click_unread_conv",
        conv_ok,
        json_str(&click_conv, "message"),
    );
    if !conv_ok {
        return Ok(XhsUnreadReadResult {
            profile_id: profile.id,
            profile_name: profile.name,
            debug_port: profile.debug_port,
            cdp_online: true,
            tab_id: target.id,
            tab_title: target.title,
            tab_url: target.url,
            nav_badge,
            clicked_nav: true,
            conversations,
            opened_index,
            peer_name,
            messages: vec![],
            replied: false,
            reply_text: String::new(),
            steps,
            message: json_str(&click_conv, "message"),
            probed_at,
        });
    }
    thread::sleep(Duration::from_millis(600));

    // 5) wait msg list
    let wait_msg = cdp_runtime_evaluate(&ws, XHS_WAIT_MSG_LIST_JS, 15)?;
    let msg_ready = json_bool(&wait_msg, "ok");
    push_step(
        &mut steps,
        "wait_msg_list",
        msg_ready,
        json_str(&wait_msg, "message"),
    );
    if !msg_ready {
        return Ok(XhsUnreadReadResult {
            profile_id: profile.id,
            profile_name: profile.name,
            debug_port: profile.debug_port,
            cdp_online: true,
            tab_id: target.id,
            tab_title: target.title,
            tab_url: target.url,
            nav_badge,
            clicked_nav: true,
            conversations,
            opened_index,
            peer_name,
            messages: vec![],
            replied: false,
            reply_text: String::new(),
            steps,
            message: json_str(&wait_msg, "message"),
            probed_at,
        });
    }
    thread::sleep(Duration::from_millis(300));

    // 6) scrape
    let scrape = cdp_runtime_evaluate(&ws, XHS_SCRAPE_MESSAGES_JS, 8)?;
    let scrape_ok = json_bool(&scrape, "ok");
    let mut messages: Vec<XhsChatMessage> = Vec::new();
    if let Some(arr) = scrape.get("messages").and_then(|v| v.as_array()) {
        for m in arr {
            messages.push(XhsChatMessage {
                direction: json_str(m, "direction"),
                text: json_str(m, "text"),
            });
        }
    }
    let scrape_peer = json_str(&scrape, "peerName");
    if !scrape_peer.is_empty() {
        peer_name = scrape_peer;
    }
    push_step(
        &mut steps,
        "scrape_messages",
        scrape_ok,
        format!(
            "{} · 昵称={}",
            json_str(&scrape, "message"),
            if peer_name.is_empty() {
                "-"
            } else {
                &peer_name
            }
        ),
    );

    // Reply is owned by front-end Agent takeover (no built-in polite phrase).
    let replied = false;
    let reply_text = String::new();
    if scrape_ok {
        push_step(
            &mut steps,
            "reply_deferred",
            true,
            "已抓取消息，回复交由自动运营客服 Agent",
        );
    }

    let summary = if scrape_ok {
        format!(
            "已读未读会话 index={:?}，共 {} 条消息（待 Agent 回复）{}",
            opened_index,
            messages.len(),
            if peer_name.is_empty() {
                String::new()
            } else {
                format!(" · {}", peer_name)
            }
        )
    } else {
        json_str(&scrape, "message")
    };

    Ok(XhsUnreadReadResult {
        profile_id: profile.id,
        profile_name: profile.name,
        debug_port: profile.debug_port,
        cdp_online: true,
        tab_id: target.id,
        tab_title: target.title,
        tab_url: target.url,
        nav_badge,
        clicked_nav: true,
        conversations,
        opened_index,
        peer_name,
        messages,
        replied,
        reply_text,
        steps,
        message: summary,
        probed_at,
    })
}

/// Probe IM stay state: current chat tail + unread conv badges (no clicks).
const XHS_PROBE_IM_STATE_JS: &str = r#"(function(){
  function textOf(el){ if (!el) return ''; return String(el.textContent || el.innerText || '').replace(/\s+/g,' ').trim(); }
  function isActiveConv(item){
    if (!item) return false;
    var cn = item.className || '';
    if (/\b(active|is-active|selected|current|is-selected)\b/i.test(cn)) return true;
    if (item.getAttribute('aria-selected') === 'true') return true;
    if (item.getAttribute('data-active') === 'true') return true;
    var mark = item.querySelector('[class*="active"], [class*="selected"], [class*="current"]');
    return !!mark;
  }
  function convNick(item){
    if (!item) return '';
    var nameEl = item.querySelector('.xhs-im-conv-item__name')
      || item.querySelector('.xhs-im-conv-item__title')
      || item.querySelector('.xhs-im-conv-item__top')
      || item.querySelector('[class*="nickname"]')
      || item.querySelector('[class*="user-name"]');
    var t = textOf(nameEl);
    if (!t) return '';
    return t.replace(/\s+\d+\+?\s*$/,'').trim().slice(0, 40);
  }
  function peerNameFromDom(){
    var header = document.querySelector('.xhs-im-chat-header__name')
      || document.querySelector('.xhs-im-header__title')
      || document.querySelector('.xhs-im-view__header [class*="name"]')
      || document.querySelector('.xhs-im-chat-header [class*="name"]')
      || document.querySelector('[class*="im-chat-header"] [class*="name"]');
    var ht = textOf(header);
    if (ht) return ht.replace(/\s+\d+\+?\s*$/,'').trim().slice(0, 40);
    var root = document.querySelector('.xhs-im-conv-list__scroll') || document;
    var items = Array.prototype.slice.call(root.querySelectorAll('.xhs-im-conv-item'));
    for (var k=0;k<items.length;k++){
      if (!isActiveConv(items[k])) continue;
      var t = convNick(items[k]);
      if (t) return t;
    }
    return '';
  }
  var convScroll = document.querySelector('.xhs-im-conv-list__scroll');
  var msgList = document.querySelector('.xhs-im-view__main .xhs-im-msg-list-wrap .xhs-im-msg-list')
    || document.querySelector('.xhs-im-msg-list');
  var inIm = !!(convScroll || msgList || document.querySelector('.xhs-im-input-bar-wrap'));
  var inChat = !!msgList;
  var messages = [];
  if (msgList) {
    var items = Array.prototype.slice.call(msgList.querySelectorAll('.chat-item'));
    for (var i=0;i<items.length;i++){
      var item = items[i];
      var direction = 'unknown';
      if (item.querySelector('.chat-item__bubble-row--left')
          || item.querySelector('.chat-item__bubble-row.chat-item__bubble-row--left')) {
        direction = 'recv';
      } else if (item.querySelector('.chat-item__content--right')
          || item.querySelector('.chat-item__content.chat-item__content--right')) {
        direction = 'sent';
      }
      var bubble = item.querySelector('.xhs-im-bubble__text');
      var text = bubble ? textOf(bubble) : textOf(item);
      if (!text) continue;
      messages.push({ direction:direction, text:text });
    }
    if (messages.length > 40) messages = messages.slice(messages.length - 40);
  }
  var last = messages.length ? messages[messages.length - 1] : null;
  var fingerprint = last ? (last.direction + '|' + last.text) : '';
  var unreadConversations = [];
  var root = convScroll || document;
  var convItems = Array.prototype.slice.call(root.querySelectorAll('.xhs-im-conv-item'));
  var activeConvIndex = -1;
  var activeConvName = '';
  for (var j=0;j<convItems.length;j++){
    var c = convItems[j];
    var active = isActiveConv(c);
    var nick = convNick(c);
    if (active) {
      activeConvIndex = j;
      activeConvName = nick;
    }
    var badgeEl = c.querySelector('.xhs-im-conv-item__badge');
    var badgeRaw = badgeEl ? textOf(badgeEl) : '';
    var badgeNum = parseInt(badgeRaw, 10);
    var badge = isNaN(badgeNum) ? 0 : badgeNum;
    if (badge <= 0) continue;
    var previewEl = c.querySelector('.xhs-im-conv-item__bottom')
      || c.querySelector('.xhs-im-conv-item__content') || c;
    unreadConversations.push({
      index: j,
      badge: badge,
      preview: textOf(previewEl).slice(0, 80),
      name: nick,
      active: active
    });
  }
  var peerName = peerNameFromDom() || activeConvName;
  return {
    ok: true,
    inIm: inIm,
    inChat: inChat,
    peerName: peerName,
    activeConvIndex: activeConvIndex,
    activeConvName: activeConvName || peerName,
    messageCount: messages.length,
    messages: messages,
    lastDirection: last ? last.direction : '',
    lastText: last ? last.text : '',
    fingerprint: fingerprint,
    unreadConversations: unreadConversations,
    message: inIm ? (inChat ? '在对话框内' : '在消息页但未打开对话框') : '不在消息页'
  };
})()"#;

const XHS_CLICK_CONV_BY_INDEX_JS_PREFIX: &str = r#"(function(){
  var index = "#;

const XHS_CLICK_CONV_BY_INDEX_JS_SUFFIX: &str = r#";
  function textOf(el){ if (!el) return ''; return String(el.textContent || el.innerText || '').replace(/\s+/g,' ').trim(); }
  function convNick(item){
    if (!item) return '';
    var nameEl = item.querySelector('.xhs-im-conv-item__name')
      || item.querySelector('.xhs-im-conv-item__title')
      || item.querySelector('.xhs-im-conv-item__top')
      || item.querySelector('[class*="nickname"]')
      || item.querySelector('[class*="user-name"]');
    var t = textOf(nameEl);
    if (!t) return '';
    return t.replace(/\s+\d+\+?\s*$/,'').trim().slice(0, 40);
  }
  var root = document.querySelector('.xhs-im-conv-list__scroll') || document;
  var items = Array.prototype.slice.call(root.querySelectorAll('.xhs-im-conv-item'));
  if (index < 0 || index >= items.length) {
    return { ok:false, index:index, peerName:'', message:'会话 index 越界: ' + index + '/' + items.length };
  }
  var peerName = convNick(items[index]);
  try { items[index].click(); } catch (e) {
    return {
      ok:false, index:index, peerName:peerName,
      message:'点击会话失败: ' + (e && e.message ? e.message : String(e))
    };
  }
  return { ok:true, index:index, peerName:peerName, message:'已点击会话 index=' + index };
})()"#;

fn resolve_xhs_cdp_ws(
    db: &State<'_, crate::library::LibraryState>,
    profile_id: Option<String>,
) -> Result<(BrowserProfile, CdpTarget, String), String> {
    let id = resolve_profile_id(profile_id);
    let conn = db.conn.lock().map_err(|_| "db lock".to_string())?;
    let profile = load_profile(&conn, &id)?.ok_or_else(|| format!("Profile 不存在: {id}"))?;
    drop(conn);
    if fetch_cdp_version(profile.debug_port).is_none() {
        return Err(format!("CDP 离线（端口 {}）", profile.debug_port));
    }
    let targets = fetch_cdp_targets_raw(profile.debug_port)?;
    let target = pick_xhs_page_target(&targets, None)?;
    let ws = target.web_socket_debugger_url.clone();
    Ok((profile, target, ws))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsImUnreadConv {
    pub index: usize,
    pub badge: i64,
    pub preview: String,
    #[serde(default)]
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsImStateProbe {
    pub profile_id: String,
    pub profile_name: String,
    pub debug_port: u16,
    pub cdp_online: bool,
    pub tab_id: String,
    pub tab_title: String,
    pub tab_url: String,
    pub in_im: bool,
    pub in_chat: bool,
    pub peer_name: String,
    pub active_conv_index: i64,
    pub active_conv_name: String,
    pub message_count: usize,
    pub messages: Vec<XhsChatMessage>,
    pub last_direction: String,
    pub last_text: String,
    pub fingerprint: String,
    pub unread_conversations: Vec<XhsImUnreadConv>,
    pub message: String,
    pub probed_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsImActionResult {
    pub ok: bool,
    pub reply_text: String,
    pub message: String,
    #[serde(default)]
    pub peer_name: String,
    #[serde(default)]
    pub index: Option<usize>,
    pub tab_id: String,
    pub probed_at: i64,
}

const XHS_PROBE_CONV_LIST_JS: &str = r#"(function(){
  function textOf(el){ if (!el) return ''; return String(el.textContent || el.innerText || '').replace(/\s+/g,' ').trim(); }
  function isActiveConv(item){
    if (!item) return false;
    var cn = item.className || '';
    if (/\b(active|is-active|selected|current|is-selected)\b/i.test(cn)) return true;
    if (item.getAttribute('aria-selected') === 'true') return true;
    if (item.getAttribute('data-active') === 'true') return true;
    return !!item.querySelector('[class*="active"], [class*="selected"], [class*="current"]');
  }
  function convNick(item){
    if (!item) return '';
    var nameEl = item.querySelector('.xhs-im-conv-item__name')
      || item.querySelector('.xhs-im-conv-item__title')
      || item.querySelector('.xhs-im-conv-item__top')
      || item.querySelector('[class*="nickname"]')
      || item.querySelector('[class*="user-name"]');
    var t = textOf(nameEl);
    if (!t) return '';
    return t.replace(/\s+\d+\+?\s*$/,'').trim().slice(0, 40);
  }
  var convScroll = document.querySelector('.xhs-im-conv-list__scroll');
  var inIm = !!(convScroll || document.querySelector('.xhs-im-msg-list') || document.querySelector('.xhs-im-input-bar-wrap'));
  var root = convScroll || document;
  var items = Array.prototype.slice.call(root.querySelectorAll('.xhs-im-conv-item'));
  var conversations = [];
  for (var i=0;i<items.length;i++){
    var c = items[i];
    var nick = convNick(c);
    var badgeEl = c.querySelector('.xhs-im-conv-item__badge');
    var badgeRaw = badgeEl ? textOf(badgeEl) : '';
    var badgeNum = parseInt(badgeRaw, 10);
    var badge = isNaN(badgeNum) ? 0 : badgeNum;
    var previewEl = c.querySelector('.xhs-im-conv-item__bottom')
      || c.querySelector('.xhs-im-conv-item__content') || c;
    var preview = textOf(previewEl).slice(0, 80);
    conversations.push({
      index: i,
      name: nick || ('会话' + (i + 1)),
      badge: badge,
      preview: preview,
      active: isActiveConv(c)
    });
  }
  return {
    ok: true,
    inIm: inIm,
    conversations: conversations,
    message: inIm ? ('会话 ' + conversations.length + ' 个') : '不在消息页'
  };
})()"#;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsConvListItem {
    pub index: usize,
    pub name: String,
    pub badge: i64,
    pub preview: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsConvListProbe {
    pub profile_id: String,
    pub profile_name: String,
    pub debug_port: u16,
    pub cdp_online: bool,
    pub tab_id: String,
    pub in_im: bool,
    pub conversations: Vec<XhsConvListItem>,
    pub message: String,
    pub probed_at: i64,
}

/// Probe IM conversation list (all items, not only unread).
#[tauri::command]
pub fn probe_xhs_conv_list(
    db: State<'_, crate::library::LibraryState>,
    profile_id: Option<String>,
) -> Result<XhsConvListProbe, String> {
    let probed_at = now_unix_secs();
    let (profile, target, ws) = match resolve_xhs_cdp_ws(&db, profile_id) {
        Ok(v) => v,
        Err(e) => {
            let id = resolve_profile_id(None);
            return Ok(XhsConvListProbe {
                profile_id: id,
                profile_name: String::new(),
                debug_port: 0,
                cdp_online: false,
                tab_id: String::new(),
                in_im: false,
                conversations: vec![],
                message: e,
                probed_at,
            });
        }
    };
    let value = cdp_runtime_evaluate(&ws, XHS_PROBE_CONV_LIST_JS, 10)?;
    let mut conversations: Vec<XhsConvListItem> = Vec::new();
    if let Some(arr) = value.get("conversations").and_then(|v| v.as_array()) {
        for c in arr {
            conversations.push(XhsConvListItem {
                index: c.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                name: json_str(c, "name"),
                badge: json_i64(c, "badge").unwrap_or(0),
                preview: json_str(c, "preview"),
                active: json_bool(c, "active"),
            });
        }
    }
    Ok(XhsConvListProbe {
        profile_id: profile.id,
        profile_name: profile.name,
        debug_port: profile.debug_port,
        cdp_online: true,
        tab_id: target.id,
        in_im: json_bool(&value, "inIm"),
        conversations,
        message: json_str(&value, "message"),
        probed_at,
    })
}

/// Probe current IM page state (no navigation / no send).
#[tauri::command]
pub fn probe_xhs_im_state(
    db: State<'_, crate::library::LibraryState>,
    profile_id: Option<String>,
) -> Result<XhsImStateProbe, String> {
    let probed_at = now_unix_secs();
    let (profile, target, ws) = match resolve_xhs_cdp_ws(&db, profile_id) {
        Ok(v) => v,
        Err(e) => {
            let id = resolve_profile_id(None);
            return Ok(XhsImStateProbe {
                profile_id: id,
                profile_name: String::new(),
                debug_port: 0,
                cdp_online: false,
                tab_id: String::new(),
                tab_title: String::new(),
                tab_url: String::new(),
                in_im: false,
                in_chat: false,
                peer_name: String::new(),
                active_conv_index: -1,
                active_conv_name: String::new(),
                message_count: 0,
                messages: vec![],
                last_direction: String::new(),
                last_text: String::new(),
                fingerprint: String::new(),
                unread_conversations: vec![],
                message: e,
                probed_at,
            });
        }
    };

    let value = cdp_runtime_evaluate(&ws, XHS_PROBE_IM_STATE_JS, 10)?;
    let mut messages: Vec<XhsChatMessage> = Vec::new();
    if let Some(arr) = value.get("messages").and_then(|v| v.as_array()) {
        for m in arr {
            messages.push(XhsChatMessage {
                direction: json_str(m, "direction"),
                text: json_str(m, "text"),
            });
        }
    }
    let mut unread_conversations: Vec<XhsImUnreadConv> = Vec::new();
    if let Some(arr) = value.get("unreadConversations").and_then(|v| v.as_array()) {
        for c in arr {
            unread_conversations.push(XhsImUnreadConv {
                index: c.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                badge: json_i64(c, "badge").unwrap_or(0),
                preview: json_str(c, "preview"),
                name: json_str(c, "name"),
                active: json_bool(c, "active"),
            });
        }
    }

    Ok(XhsImStateProbe {
        profile_id: profile.id,
        profile_name: profile.name,
        debug_port: profile.debug_port,
        cdp_online: true,
        tab_id: target.id,
        tab_title: target.title,
        tab_url: target.url,
        in_im: json_bool(&value, "inIm"),
        in_chat: json_bool(&value, "inChat"),
        peer_name: json_str(&value, "peerName"),
        active_conv_index: json_i64(&value, "activeConvIndex").unwrap_or(-1),
        active_conv_name: json_str(&value, "activeConvName"),
        message_count: value
            .get("messageCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(messages.len() as u64) as usize,
        messages,
        last_direction: json_str(&value, "lastDirection"),
        last_text: json_str(&value, "lastText"),
        fingerprint: json_str(&value, "fingerprint"),
        unread_conversations,
        message: json_str(&value, "message"),
        probed_at,
    })
}

/// Click a conversation item by index in the IM list.
#[tauri::command]
pub fn click_xhs_conv_item(
    db: State<'_, crate::library::LibraryState>,
    profile_id: Option<String>,
    index: usize,
) -> Result<XhsImActionResult, String> {
    let probed_at = now_unix_secs();
    let (_profile, target, ws) = resolve_xhs_cdp_ws(&db, profile_id)?;
    let js = format!("{XHS_CLICK_CONV_BY_INDEX_JS_PREFIX}{index}{XHS_CLICK_CONV_BY_INDEX_JS_SUFFIX}");
    let value = cdp_runtime_evaluate(&ws, &js, 8)?;
    Ok(XhsImActionResult {
        ok: json_bool(&value, "ok"),
        reply_text: String::new(),
        message: json_str(&value, "message"),
        peer_name: json_str(&value, "peerName"),
        index: value
            .get("index")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize),
        tab_id: target.id,
        probed_at,
    })
}

/// Send a polite (or custom) reply in the current chat input + Enter.
#[tauri::command]
pub fn send_xhs_chat_reply(
    db: State<'_, crate::library::LibraryState>,
    profile_id: Option<String>,
    text: Option<String>,
) -> Result<XhsImActionResult, String> {
    let probed_at = now_unix_secs();
    let (_profile, target, ws) = resolve_xhs_cdp_ws(&db, profile_id)?;
    let phrase = text
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| pick_polite_reply().to_string());
    let text_json = serde_json::to_string(&phrase).unwrap_or_else(|_| "\"您好，收到了～\"".into());
    let send_js = xhs_send_reply_js(&text_json);
    let value = cdp_runtime_evaluate(&ws, &send_js, 40)?;
    let ok = json_bool(&value, "ok");
    Ok(XhsImActionResult {
        ok,
        reply_text: if ok { phrase } else { String::new() },
        message: json_str(&value, "message"),
        peer_name: String::new(),
        index: None,
        tab_id: target.id,
        probed_at,
    })
}

// ─── AI 获客：搜索 / 筛选 / 评论回复 ─────────────────────────────────

fn cdp_send_method(
    ws_url: &str,
    method: &str,
    params: serde_json::Value,
    timeout_secs: u64,
) -> Result<serde_json::Value, String> {
    use std::net::TcpStream as StdTcp;
    use tungstenite::{client, protocol::Message};

    let timeout = timeout_secs.max(3);
    let (host, port) = parse_ws_host_port(ws_url)?;
    let addr = format!("{host}:{port}");
    let stream = StdTcp::connect_timeout(
        &addr
            .parse()
            .map_err(|e| format!("解析 CDP 地址失败: {e}"))?,
        Duration::from_secs(3),
    )
    .map_err(|e| format!("连接 CDP WebSocket 失败: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(timeout)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .ok();

    let (mut socket, _) =
        client(ws_url, stream).map_err(|e| format!("CDP WebSocket 握手失败: {e}"))?;

    let req_id = 42u64;
    let payload = serde_json::json!({
        "id": req_id,
        "method": method,
        "params": params,
    });
    socket
        .send(Message::Text(payload.to_string()))
        .map_err(|e| format!("发送 {method} 失败: {e}"))?;

    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        if Instant::now() > deadline {
            return Err(format!("等待 {method} 响应超时"));
        }
        let msg = match socket.read() {
            Ok(m) => m,
            Err(e) => return Err(format!("读取 CDP 响应失败: {e}")),
        };
        let text = match msg {
            Message::Text(t) => t,
            Message::Ping(data) => {
                let _ = socket.send(Message::Pong(data));
                continue;
            }
            Message::Close(_) => return Err("CDP WebSocket 已关闭".into()),
            _ => continue,
        };
        let body: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("解析 CDP 响应失败: {e}"))?;
        if body.get("id").and_then(|v| v.as_u64()) != Some(req_id) {
            continue;
        }
        if let Some(err) = body.get("error") {
            return Err(format!(
                "{method} 错误: {}",
                err.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&err.to_string())
            ));
        }
        return Ok(body.get("result").cloned().unwrap_or(serde_json::Value::Null));
    }
}

thread_local! {
    /// 拟人鼠标：记住上一次光标位置，便于分步移动
    static HUMAN_MOUSE_XY: Cell<(f64, f64)> = Cell::new((420.0, 360.0));
}

fn human_rand_u32() -> u32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos().wrapping_add(d.as_secs() as u32))
        .unwrap_or(1);
    nanos.wrapping_mul(1664525).wrapping_add(1013904223)
}

fn human_rand_range(min: u64, max: u64) -> u64 {
    let lo = min.min(max);
    let hi = min.max(max);
    if hi <= lo {
        return lo;
    }
    lo + (human_rand_u32() as u64) % (hi - lo + 1)
}

fn human_sleep(min_ms: u64, max_ms: u64) {
    thread::sleep(Duration::from_millis(human_rand_range(min_ms, max_ms)));
}

fn human_jitter(base: f64, amp: f64) -> f64 {
    let t = (human_rand_u32() % 1000) as f64 / 1000.0;
    base + (t * 2.0 - 1.0) * amp
}

fn cdp_mouse_moved(ws: &str, x: f64, y: f64) -> Result<(), String> {
    cdp_send_method(
        ws,
        "Input.dispatchMouseEvent",
        serde_json::json!({
            "type": "mouseMoved",
            "x": x,
            "y": y,
            "buttons": 0,
            "pointerType": "mouse",
        }),
        5,
    )
    .map(|_| {
        HUMAN_MOUSE_XY.with(|c| c.set((x, y)));
    })
}

/// 拟人分步移动：缓动 + 轻微抖动，避免瞬移
fn cdp_mouse_move_human(ws: &str, to_x: f64, to_y: f64) -> Result<(), String> {
    let (from_x, from_y) = HUMAN_MOUSE_XY.with(|c| c.get());
    let dx = to_x - from_x;
    let dy = to_y - from_y;
    let dist = (dx * dx + dy * dy).sqrt();
    let steps = ((dist / 28.0).ceil() as usize).clamp(6, 22);
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        // ease-in-out
        let e = if t < 0.5 {
            2.0 * t * t
        } else {
            -1.0 + (4.0 - 2.0 * t) * t
        };
        let x = human_jitter(from_x + dx * e, 1.6);
        let y = human_jitter(from_y + dy * e, 1.6);
        cdp_mouse_moved(ws, x, y)?;
        thread::sleep(Duration::from_millis(human_rand_range(8, 22)));
    }
    cdp_mouse_moved(ws, to_x, to_y)?;
    Ok(())
}

fn cdp_mouse_wheel(ws: &str, x: f64, y: f64, delta_y: f64) -> Result<(), String> {
    cdp_send_method(
        ws,
        "Input.dispatchMouseEvent",
        serde_json::json!({
            "type": "mouseWheel",
            "x": x,
            "y": y,
            "deltaX": 0,
            "deltaY": delta_y,
            "pointerType": "mouse",
        }),
        5,
    )
    .map(|_| ())
}

/// 拟人滚轮：拆成多段小滚 + 间隔停顿
fn cdp_mouse_wheel_human(ws: &str, x: f64, y: f64, total_delta: f64) -> Result<(), String> {
    let _ = cdp_mouse_move_human(ws, human_jitter(x, 6.0), human_jitter(y, 8.0));
    human_sleep(120, 280);
    let chunks = human_rand_range(3, 5) as i32;
    let each = total_delta / chunks as f64;
    for i in 0..chunks {
        let d = each * human_jitter(1.0, 0.12);
        cdp_mouse_wheel(ws, human_jitter(x, 4.0), human_jitter(y, 5.0), d)?;
        if i + 1 < chunks {
            human_sleep(90, 220);
        }
    }
    human_sleep(280, 560);
    Ok(())
}

fn cdp_mouse_click(ws: &str, x: f64, y: f64) -> Result<(), String> {
    cdp_mouse_click_human(ws, x, y)
}

/// 拟人点击：移入目标 → 停顿瞄准 → 按下短按 → 松开
fn cdp_mouse_click_human(ws: &str, x: f64, y: f64) -> Result<(), String> {
    let tx = human_jitter(x, 2.2);
    let ty = human_jitter(y, 2.2);
    cdp_mouse_move_human(ws, tx, ty)?;
    human_sleep(160, 420); // 瞄准停顿
    let base = serde_json::json!({
        "x": tx,
        "y": ty,
        "button": "left",
        "clickCount": 1,
        "pointerType": "mouse",
    });
    cdp_send_method(
        ws,
        "Input.dispatchMouseEvent",
        {
            let mut m = base.clone();
            if let Some(o) = m.as_object_mut() {
                o.insert("type".into(), serde_json::json!("mousePressed"));
                o.insert("buttons".into(), serde_json::json!(1));
            }
            m
        },
        5,
    )?;
    human_sleep(45, 130); // 按住时长
    cdp_send_method(
        ws,
        "Input.dispatchMouseEvent",
        {
            let mut m = base;
            if let Some(o) = m.as_object_mut() {
                o.insert("type".into(), serde_json::json!("mouseReleased"));
                o.insert("buttons".into(), serde_json::json!(0));
            }
            m
        },
        5,
    )?;
    human_sleep(180, 380);
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsFilterTag {
    pub text: String,
    pub data_hp_kind: String,
    pub aria_hidden: bool,
    pub opacity: String,
    pub visible: bool,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsFilterPanelProbe {
    pub ok: bool,
    pub profile_id: String,
    pub tab_id: String,
    pub filter_found: bool,
    pub panel_found: bool,
    pub tags: Vec<XhsFilterTag>,
    pub message: String,
    pub probed_at: i64,
    pub steps: Vec<XhsUnreadStep>,
}

const XHS_HOVER_FILTER_AND_DUMP_JS: &str = r#"(async function(){
  function sleep(ms){ return new Promise(function(r){ setTimeout(r, ms); }); }
  function visible(el){
    if (!el) return false;
    var st = window.getComputedStyle(el);
    if (st.display === 'none' || st.visibility === 'hidden') return false;
    var op = parseFloat(st.opacity || '1');
    if (isNaN(op) || op < 0.05) return false;
    var r = el.getBoundingClientRect();
    return r.width > 2 && r.height > 2;
  }
  var filter = document.querySelector('.filter.ai-chat-filter')
    || document.querySelector('.ai-chat-filter')
    || document.querySelector('[class*="ai-chat-filter"]');
  if (!filter) {
    return { ok:false, filterFound:false, panelFound:false, filterRect:null, tags:[], message:'未找到 .filter.ai-chat-filter' };
  }
  var fr = filter.getBoundingClientRect();
  var cx = fr.left + fr.width / 2;
  var cy = fr.top + fr.height / 2;
  try {
    filter.dispatchEvent(new MouseEvent('mouseenter', { bubbles:true, clientX:cx, clientY:cy }));
    filter.dispatchEvent(new MouseEvent('mouseover', { bubbles:true, clientX:cx, clientY:cy }));
  } catch (e) {}
  await sleep(350);
  var panel = document.querySelector('.filter-panel');
  var tags = [];
  if (panel) {
    var nodes = Array.prototype.slice.call(panel.querySelectorAll('.tags'));
    for (var i=0;i<nodes.length;i++){
      var el = nodes[i];
      var st = window.getComputedStyle(el);
      var r = el.getBoundingClientRect();
      var text = (el.innerText || el.textContent || '').replace(/\s+/g,' ').trim();
      var aria = el.getAttribute('aria-hidden') === 'true';
      var op = st.opacity || '1';
      var z = parseInt(st.zIndex || '0', 10);
      var vis = visible(el) && !aria && !(z < 0);
      tags.push({
        text: text,
        dataHpKind: el.getAttribute('data-hp-kind') || '',
        ariaHidden: aria,
        opacity: op,
        visible: vis,
        x: r.left + r.width / 2,
        y: r.top + r.height / 2,
        w: r.width,
        h: r.height
      });
    }
  }
  return {
    ok: true,
    filterFound: true,
    panelFound: !!panel,
    filterRect: { x: cx, y: cy, w: fr.width, h: fr.height },
    tags: tags,
    message: panel ? ('面板已打开，tags=' + tags.length) : '已 hover，但未出现 .filter-panel'
  };
})()"#;

#[tauri::command]
pub fn probe_xhs_filter_panel(
    db: State<'_, crate::library::LibraryState>,
    profile_id: Option<String>,
) -> Result<XhsFilterPanelProbe, String> {
    let probed_at = now_unix_secs();
    let mut steps = Vec::new();
    let (profile, target, ws) = resolve_xhs_cdp_ws(&db, profile_id)?;
    // CDP 坐标 hover 一次（辅助触发）
    let pre = cdp_runtime_evaluate(
        &ws,
        r#"(function(){
          var f = document.querySelector('.filter.ai-chat-filter') || document.querySelector('.ai-chat-filter');
          if (!f) return { ok:false };
          var r = f.getBoundingClientRect();
          return { ok:true, x: r.left + r.width/2, y: r.top + r.height/2 };
        })()"#,
        8,
    )
    .unwrap_or(serde_json::Value::Null);
    if json_bool(&pre, "ok") {
        let x = pre.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let y = pre.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let _ = cdp_mouse_moved(&ws, x, y);
        push_step(&mut steps, "cdp_hover", true, format!("mouseMoved ({x:.0},{y:.0})"));
        thread::sleep(Duration::from_millis(200));
    } else {
        push_step(&mut steps, "cdp_hover", false, "定位筛选触发器失败，改走 JS hover");
    }
    let value = cdp_runtime_evaluate(&ws, XHS_HOVER_FILTER_AND_DUMP_JS, 12)?;
    let mut tags = Vec::new();
    if let Some(arr) = value.get("tags").and_then(|v| v.as_array()) {
        for t in arr {
            tags.push(XhsFilterTag {
                text: json_str(t, "text"),
                data_hp_kind: json_str(t, "dataHpKind"),
                aria_hidden: json_bool(t, "ariaHidden"),
                opacity: json_str(t, "opacity"),
                visible: json_bool(t, "visible"),
                x: t.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0),
                y: t.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0),
                w: t.get("w").and_then(|v| v.as_f64()).unwrap_or(0.0),
                h: t.get("h").and_then(|v| v.as_f64()).unwrap_or(0.0),
            });
        }
    }
    let ok = json_bool(&value, "ok");
    push_step(&mut steps, "dump_panel", ok, json_str(&value, "message"));
    Ok(XhsFilterPanelProbe {
        ok,
        profile_id: profile.id,
        tab_id: target.id,
        filter_found: json_bool(&value, "filterFound"),
        panel_found: json_bool(&value, "panelFound"),
        tags,
        message: json_str(&value, "message"),
        probed_at,
        steps,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsCommentMatchProbeBody {
    pub profile_id: Option<String>,
    /// 评论匹配关键词（多个用 @ 分隔）
    pub comment_keyword: String,
    /// 对评论滑动区滚轮次数（默认 5）
    pub max_scrolls: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsMatchedComment {
    pub index: i64,
    pub text: String,
    pub matched_keyword: String,
    pub has_reply_icon: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsCommentMatchProbe {
    pub ok: bool,
    pub profile_id: String,
    pub tab_id: String,
    pub scroller_found: bool,
    pub comments_found: bool,
    pub scroller_selector: String,
    pub scroller_x: f64,
    pub scroller_y: f64,
    pub total_comments: i64,
    pub matched: Vec<XhsMatchedComment>,
    pub scrolls_done: i64,
    pub message: String,
    pub probed_at: i64,
    pub steps: Vec<XhsUnreadStep>,
}

/// 定位当前已打开笔记的评论滑动区（.note-content / .comments-container）
const XHS_LOCATE_COMMENT_SCROLLER_JS: &str = r#"(function(){
  function pick(){
    var candidates = [
      '.note-content',
      '.comments-container',
      '#noteContainer .note-content',
      '#noteContainer .comments-container',
      '.note-detail-mask .note-content',
      '.note-scroller'
    ];
    for (var i=0;i<candidates.length;i++){
      var el = document.querySelector(candidates[i]);
      if (!el) continue;
      var r = el.getBoundingClientRect();
      if (r.width < 40 || r.height < 40) continue;
      var st = window.getComputedStyle(el);
      if (st.display === 'none' || st.visibility === 'hidden') continue;
      return { el: el, selector: candidates[i], rect: r };
    }
    return null;
  }
  var hit = pick();
  if (!hit) {
    return {
      ok:false, scrollerFound:false, commentsFound:false,
      selector:'', x:0, y:0, w:0, h:0, total:0,
      message:'未找到评论滑动区（请先在浏览器中打开一篇笔记详情）'
    };
  }
  var box = document.querySelector('.comments-container')
    || document.querySelector('#noteContainer .comments-container');
  var total = box ? box.querySelectorAll('.parent-comment').length : 0;
  var r = hit.rect;
  var x = r.left + r.width / 2;
  var y = r.top + Math.min(r.height * 0.72, r.height - 24);
  return {
    ok:true,
    scrollerFound:true,
    commentsFound: !!box,
    selector: hit.selector,
    x: x, y: y, w: r.width, h: r.height,
    scrollTop: hit.el.scrollTop || 0,
    scrollHeight: hit.el.scrollHeight || 0,
    total: total,
    message: '已定位 ' + hit.selector + ' · parent-comment=' + total
  };
})()"#;

/// 评论关键词：用 `@` 区分多个（含任一即命中）。
fn parse_comment_keywords(raw: &str) -> Vec<String> {
    raw.split('@')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn xhs_probe_match_comments_js(keywords_json: &str) -> String {
    format!(
        r#"(function(){{
  var keywords = {keywords_json};
  function norm(s){{ return String(s || '').toLowerCase(); }}
  function hitKw(text){{
    var t = norm(text);
    for (var i=0;i<keywords.length;i++){{
      var k = norm(keywords[i]);
      if (k && t.indexOf(k) >= 0) return keywords[i];
    }}
    return '';
  }}
  var box = document.querySelector('.comments-container')
    || document.querySelector('#noteContainer .comments-container');
  if (!box) return {{ ok:false, total:0, matched:[], message:'无 .comments-container' }};
  var parents = Array.prototype.slice.call(box.querySelectorAll('.parent-comment'));
  var matched = [];
  for (var i=0;i<parents.length;i++){{
    var pc = parents[i];
    var note = pc.querySelector('.note-text');
    var text = note ? (note.innerText || note.textContent || '') : (pc.innerText || '');
    text = String(text).replace(/\s+/g,' ').trim();
    var kw = hitKw(text);
    if (!kw) continue;
    var icon = pc.querySelector('.reds-icon.reply-icon')
      || pc.querySelector('svg.reds-icon.reply-icon')
      || pc.querySelector('.interactions .reds-icon.reply-icon');
    matched.push({{
      index: i,
      text: text.slice(0, 120),
      matchedKeyword: kw,
      hasReplyIcon: !!icon
    }});
  }}
  return {{
    ok:true,
    total: parents.length,
    matched: matched,
    message: '扫描 ' + parents.length + ' 条，命中 ' + matched.length + ' 条'
  }};
}})()"#
    )
}

/// 在当前已打开的笔记详情中：鼠标定位评论滑动区 → 滚轮加载 → 按关键词匹配评论（不发送回复）
#[tauri::command]
pub fn probe_xhs_comment_match(
    db: State<'_, crate::library::LibraryState>,
    body: XhsCommentMatchProbeBody,
) -> Result<XhsCommentMatchProbe, String> {
    let probed_at = now_unix_secs();
    let mut steps = Vec::new();
    let comment_kw_raw = body.comment_keyword.trim().to_string();
    if comment_kw_raw.is_empty() {
        return Err("请填写评论匹配关键词".into());
    }
    let max_scrolls = body.max_scrolls.unwrap_or(5).clamp(0, 10);
    let match_keywords = parse_comment_keywords(&comment_kw_raw);
    if match_keywords.is_empty() {
        return Err("请填写评论匹配关键词".into());
    }
    let kw_arr = serde_json::to_string(&match_keywords).unwrap_or_else(|_| "[]".into());

    let (profile, target, ws) = resolve_xhs_cdp_ws(&db, body.profile_id)?;

    // 1) 定位评论滑动区
    let locate = cdp_runtime_evaluate(&ws, XHS_LOCATE_COMMENT_SCROLLER_JS, 10)?;
    let scroller_found = json_bool(&locate, "scrollerFound");
    let comments_found = json_bool(&locate, "commentsFound");
    let selector = json_str(&locate, "selector");
    let mut x = locate.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let mut y = locate.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    push_step(
        &mut steps,
        "locate_scroller",
        scroller_found,
        json_str(&locate, "message"),
    );
    if !scroller_found {
        return Ok(XhsCommentMatchProbe {
            ok: false,
            profile_id: profile.id,
            tab_id: target.id,
            scroller_found: false,
            comments_found: false,
            scroller_selector: String::new(),
            scroller_x: 0.0,
            scroller_y: 0.0,
            total_comments: 0,
            matched: vec![],
            scrolls_done: 0,
            message: json_str(&locate, "message"),
            probed_at,
            steps,
        });
    }

    // 2) 拟人鼠标移入评论滑动区
    let hover_ok = cdp_mouse_move_human(&ws, x, y).is_ok();
    push_step(
        &mut steps,
        "mouse_to_scroller",
        hover_ok,
        format!("拟人移入 ({x:.0},{y:.0}) → {selector}"),
    );
    human_sleep(350, 700);

    // 3) 拟人滚轮下滑加载更多 + 匹配
    let mut scrolls_done = 0i64;
    let mut best_total = 0i64;
    let mut matched: Vec<XhsMatchedComment> = Vec::new();
    let match_js = xhs_probe_match_comments_js(&kw_arr);

    for round in 0..=max_scrolls {
        if round > 0 {
            // 刷新坐标（布局可能变）
            if let Ok(loc2) = cdp_runtime_evaluate(&ws, XHS_LOCATE_COMMENT_SCROLLER_JS, 8) {
                if json_bool(&loc2, "scrollerFound") {
                    x = loc2.get("x").and_then(|v| v.as_f64()).unwrap_or(x);
                    y = loc2.get("y").and_then(|v| v.as_f64()).unwrap_or(y);
                }
            }
            let wheel_ok = cdp_mouse_wheel_human(&ws, x, y, human_jitter(560.0, 120.0)).is_ok();
            // JS 兜底滚动
            let _ = cdp_runtime_evaluate(
                &ws,
                r#"(function(){
                  var el = document.querySelector('.note-content')
                    || document.querySelector('.comments-container')
                    || document.querySelector('#noteContainer');
                  if (!el) return { ok:false };
                  el.scrollTop = (el.scrollTop || 0) + Math.max(280, el.clientHeight * 0.7);
                  el.dispatchEvent(new Event('scroll', { bubbles:true }));
                  return { ok:true, scrollTop: el.scrollTop };
                })()"#,
                8,
            );
            scrolls_done += 1;
            push_step(
                &mut steps,
                &format!("wheel_{scrolls_done}"),
                wheel_ok,
                format!("拟人滚轮下滑 #{scrolls_done}"),
            );
            human_sleep(700, 1300);
        }

        let scan = cdp_runtime_evaluate(&ws, &match_js, 12)?;
        let total = scan.get("total").and_then(|v| v.as_i64()).unwrap_or(0);
        best_total = best_total.max(total);
        let mut round_matched = Vec::new();
        if let Some(arr) = scan.get("matched").and_then(|v| v.as_array()) {
            for m in arr {
                round_matched.push(XhsMatchedComment {
                    index: m.get("index").and_then(|v| v.as_i64()).unwrap_or(-1),
                    text: json_str(m, "text"),
                    matched_keyword: json_str(m, "matchedKeyword"),
                    has_reply_icon: json_bool(m, "hasReplyIcon"),
                });
            }
        }
        // 合并去重（按 text）
        for m in round_matched {
            if !matched.iter().any(|x| x.text == m.text && x.index == m.index) {
                matched.push(m);
            }
        }
        push_step(
            &mut steps,
            &format!("match_r{round}"),
            json_bool(&scan, "ok"),
            format!(
                "{}（累计命中 {}）",
                json_str(&scan, "message"),
                matched.len()
            ),
        );
    }

    let ok = comments_found || best_total > 0;
    let message = format!(
        "滑动区「{selector}」滚轮 {scrolls_done} 次 · 评论 {best_total} 条 · 命中 {} 条",
        matched.len()
    );
    Ok(XhsCommentMatchProbe {
        ok,
        profile_id: profile.id,
        tab_id: target.id,
        scroller_found,
        comments_found: comments_found || best_total > 0,
        scroller_selector: selector,
        scroller_x: x,
        scroller_y: y,
        total_comments: best_total,
        matched,
        scrolls_done,
        message,
        probed_at,
        steps,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsCustomReplyProbeBody {
    pub profile_id: Option<String>,
    pub comment_keyword: String,
    pub reply_text: String,
    /// 下滑次数，默认 5
    pub max_scrolls: Option<i64>,
    /// 探测时最多实际发送几条，默认 1
    pub max_replies: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsCustomReplyProbe {
    pub ok: bool,
    pub profile_id: String,
    pub tab_id: String,
    pub scroller_found: bool,
    pub matched: i64,
    pub sent: i64,
    pub scrolls: i64,
    pub message: String,
    pub probed_at: i64,
    pub steps: Vec<XhsUnreadStep>,
}

/// 当前已打开笔记：定位评论区 → 匹配关键词 → 点回复 → 填入自定义文案并 Enter（探测发送链路）
#[tauri::command]
pub fn probe_xhs_custom_reply(
    db: State<'_, crate::library::LibraryState>,
    body: XhsCustomReplyProbeBody,
) -> Result<XhsCustomReplyProbe, String> {
    let probed_at = now_unix_secs();
    let mut steps = Vec::new();
    let comment_kw_raw = body.comment_keyword.trim().to_string();
    if comment_kw_raw.is_empty() {
        return Err("请填写评论匹配关键词".into());
    }
    let reply_text = body.reply_text.trim().to_string();
    if reply_text.is_empty() {
        return Err("请填写自定义回复文案".into());
    }
    let max_scrolls = body.max_scrolls.unwrap_or(5).clamp(0, 10);
    let max_replies = body.max_replies.unwrap_or(1).clamp(1, 5);
    let match_keywords = parse_comment_keywords(&comment_kw_raw);
    if match_keywords.is_empty() {
        return Err("请填写评论匹配关键词".into());
    }
    let kw_arr = serde_json::to_string(&match_keywords).unwrap_or_else(|_| "[]".into());
    let reply_json = serde_json::to_string(&reply_text).unwrap_or_else(|_| "\"\"".into());

    let (profile, target, ws) = resolve_xhs_cdp_ws(&db, body.profile_id)?;

    // 1) 定位并鼠标移入评论滑动区
    let locate = cdp_runtime_evaluate(&ws, XHS_LOCATE_COMMENT_SCROLLER_JS, 10)?;
    let scroller_found = json_bool(&locate, "scrollerFound");
    push_step(
        &mut steps,
        "locate_scroller",
        scroller_found,
        json_str(&locate, "message"),
    );
    if !scroller_found {
        return Ok(XhsCustomReplyProbe {
            ok: false,
            profile_id: profile.id,
            tab_id: target.id,
            scroller_found: false,
            matched: 0,
            sent: 0,
            scrolls: 0,
            message: json_str(&locate, "message"),
            probed_at,
            steps,
        });
    }
    let x = locate.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y = locate.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let selector = json_str(&locate, "selector");
    let hover_ok = cdp_mouse_move_human(&ws, x, y).is_ok();
    push_step(
        &mut steps,
        "mouse_to_scroller",
        hover_ok,
        format!("拟人移入评论区 ({x:.0},{y:.0}) → {selector}"),
    );
    human_sleep(400, 800);

    // 2) 匹配 → 拟人点击 .reds-icon.reply-icon → 逐字填入自定义文案发送
    push_step(
        &mut steps,
        "reply_template",
        true,
        format!(
            "文案预览: {}",
            reply_text.chars().take(40).collect::<String>()
        ),
    );
    let (matched, sent, scrolls, msg) = xhs_acq_do_match_and_reply(
        None,
        &ws,
        &kw_arr,
        &reply_json,
        max_replies,
        max_scrolls,
        Some(&mut steps),
        None,
    )?;
    let reply_ok = sent > 0;
    let message = if sent > 0 {
        format!("探测成功：匹配 {matched} · 已发送自定义回复 {sent} 条（下滑 {scrolls}）· {msg}")
    } else if matched > 0 {
        format!("命中 {matched} 条但发送失败，请检查 .reds-icon.reply-icon / .content-input")
    } else {
        format!("未匹配到评论关键词（下滑 {scrolls} 次），请调整关键词或再下滑")
    };

    Ok(XhsCustomReplyProbe {
        ok: reply_ok,
        profile_id: profile.id,
        tab_id: target.id,
        scroller_found: true,
        matched,
        sent,
        scrolls,
        message,
        probed_at,
        steps,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsAcqRunBody {
    pub profile_id: Option<String>,
    /// 笔记搜索关键词
    pub keyword: String,
    /// 评论匹配关键词（多个用 @ 分隔）；缺省时回退为搜索词
    pub comment_keyword: Option<String>,
    pub sort_label: Option<String>,
    pub reply_text: String,
    pub max_notes: Option<i64>,
    pub max_replies_per_note: Option<i64>,
    /// 单篇笔记内对 `.note-content` 下滑加载更多评论的次数上限（默认 5）
    pub max_comment_scrolls: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsAcqRunResult {
    pub ok: bool,
    pub profile_id: String,
    pub tab_id: String,
    pub keyword: String,
    pub comment_keyword: String,
    pub sort_label: String,
    pub notes_opened: i64,
    pub comments_matched: i64,
    pub replies_sent: i64,
    pub steps: Vec<XhsUnreadStep>,
    pub message: String,
    pub probed_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsAcqProgressEvent {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// `xhs_acq_run` 立即返回的启动回执；长任务在后台线程执行，结束时发 `xhs-acq-finished`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XhsAcqStartResult {
    pub ok: bool,
    pub profile_id: String,
    pub tab_id: String,
    pub message: String,
}

static ACQ_JOB_RUNNING: AtomicBool = AtomicBool::new(false);

fn acq_emit(app: Option<&AppHandle>, name: &str, ok: bool, detail: &str) {
    if let Some(app) = app {
        let _ = app.emit(
            "xhs-acq-progress",
            XhsAcqProgressEvent {
                name: name.to_string(),
                ok,
                detail: detail.to_string(),
            },
        );
    }
}

fn acq_push(
    app: Option<&AppHandle>,
    steps: &mut Vec<XhsUnreadStep>,
    name: &str,
    ok: bool,
    detail: impl Into<String>,
) {
    let detail = detail.into();
    push_step(steps, name, ok, detail.clone());
    acq_emit(app, name, ok, &detail);
}

fn xhs_acq_search_js(keyword_json: &str) -> String {
    format!(
        r#"(async function(){{
  var keyword = {keyword_json};
  function sleep(ms){{ return new Promise(function(r){{ setTimeout(r, ms); }}); }}
  function dispatchKey(el, type, key, code, keyCode){{
    el.dispatchEvent(new KeyboardEvent(type, {{
      key:key, code:code, keyCode:keyCode, which:keyCode, bubbles:true, cancelable:true, composed:true
    }}));
  }}
  var wrap = document.querySelector('.textarea-wrapper');
  if (!wrap) return {{ ok:false, message:'未找到 .textarea-wrapper' }};
  try {{ wrap.click(); }} catch(e) {{}}
  await sleep(200);
  var input = wrap.querySelector('textarea, input, [contenteditable="true"], [role="textbox"]')
    || document.querySelector('.textarea-wrapper textarea, .textarea-wrapper input, .textarea-wrapper [contenteditable="true"]');
  if (!input) return {{ ok:false, message:'搜索框内未找到可输入元素' }};
  try {{ input.focus(); }} catch(e) {{}}
  try {{ input.click(); }} catch(e) {{}}
  await sleep(120);
  var isInput = input.tagName === 'TEXTAREA' || input.tagName === 'INPUT';
  if (isInput) {{
    input.value = '';
    input.dispatchEvent(new Event('input', {{ bubbles:true }}));
    input.value = keyword;
    input.dispatchEvent(new Event('input', {{ bubbles:true }}));
    input.dispatchEvent(new Event('change', {{ bubbles:true }}));
  }} else {{
    try {{ document.execCommand('selectAll', false, null); document.execCommand('delete', false, null); }} catch(e) {{}}
    input.textContent = keyword;
    try {{
      input.dispatchEvent(new InputEvent('input', {{ bubbles:true, data:keyword, inputType:'insertText' }}));
    }} catch(e) {{
      input.dispatchEvent(new Event('input', {{ bubbles:true }}));
    }}
  }}
  await sleep(250);
  dispatchKey(input, 'keydown', 'Enter', 'Enter', 13);
  await sleep(80);
  return {{ ok:true, message:'已输入关键词并回车: ' + keyword }};
}})()"#
    )
}

fn xhs_acq_click_sort_js(label_json: &str) -> String {
    format!(
        r#"(async function(){{
  var label = {label_json};
  function sleep(ms){{ return new Promise(function(r){{ setTimeout(r, ms); }}); }}
  function visible(el){{
    if (!el) return false;
    if (el.getAttribute('aria-hidden') === 'true') return false;
    var st = window.getComputedStyle(el);
    var op = parseFloat(st.opacity || '1');
    if (isNaN(op) || op < 0.05) return false;
    var z = parseInt(st.zIndex || '0', 10);
    if (!isNaN(z) && z < 0) return false;
    var r = el.getBoundingClientRect();
    return r.width > 2 && r.height > 2;
  }}
  var filter = document.querySelector('.filter.ai-chat-filter') || document.querySelector('.ai-chat-filter');
  if (!filter) return {{ ok:false, message:'未找到筛选触发器' }};
  var fr = filter.getBoundingClientRect();
  var cx = fr.left + fr.width/2, cy = fr.top + fr.height/2;
  filter.dispatchEvent(new MouseEvent('mouseenter', {{ bubbles:true, clientX:cx, clientY:cy }}));
  filter.dispatchEvent(new MouseEvent('mouseover', {{ bubbles:true, clientX:cx, clientY:cy }}));
  await sleep(400);
  var panel = document.querySelector('.filter-panel');
  if (!panel) return {{ ok:false, message:'未出现 .filter-panel' }};
  var nodes = Array.prototype.slice.call(panel.querySelectorAll('.tags'));
  var hit = null;
  for (var i=0;i<nodes.length;i++){{
    var el = nodes[i];
    if (!visible(el)) continue;
    var text = (el.innerText || el.textContent || '').replace(/\s+/g,' ').trim();
    if (text === label || text.indexOf(label) >= 0) {{ hit = el; break; }}
  }}
  if (!hit) {{
    return {{ ok:false, message:'未找到可见排序项: ' + label, available: nodes.map(function(n){{ return (n.innerText||'').trim(); }}) }};
  }}
  try {{ hit.click(); }} catch(e) {{
    return {{ ok:false, message:'点击排序失败: ' + (e && e.message ? e.message : String(e)) }};
  }}
  await sleep(500);
  return {{ ok:true, message:'已选择排序: ' + label }};
}})()"#
    )
}

const XHS_ACQ_LIST_NOTES_JS: &str = r#"(function(){
  var root = document.querySelector('.feeds-container');
  if (!root) return { ok:false, count:0, message:'未找到 .feeds-container' };
  var items = Array.prototype.slice.call(root.querySelectorAll('.note-item'));
  return { ok:true, count: items.length, message:'笔记卡片 ' + items.length + ' 个' };
})()"#;

fn xhs_acq_click_note_js(index: usize) -> String {
    format!(
        r#"(async function(){{
  var index = {index};
  function sleep(ms){{ return new Promise(function(r){{ setTimeout(r, ms); }}); }}
  var root = document.querySelector('.feeds-container');
  if (!root) return {{ ok:false, message:'未找到 .feeds-container' }};
  var items = Array.prototype.slice.call(root.querySelectorAll('.note-item'));
  if (index < 0 || index >= items.length) return {{ ok:false, message:'note index 越界 ' + index + '/' + items.length }};
  var card = items[index];
  // 小红书实际可点区域是封面遮罩，点外层 note-item 常打不开详情
  var el = card.querySelector('.cover.mask.ld')
    || card.querySelector('.cover.mask')
    || card.querySelector('a.cover')
    || card.querySelector('.cover')
    || card;
  try {{ card.scrollIntoView({{ block:'center', inline:'nearest' }}); }} catch(e) {{}}
  await sleep(200);
  try {{ el.click(); }} catch(e) {{
    return {{ ok:false, message:'点击笔记失败: ' + (e && e.message ? e.message : String(e)) }};
  }}
  await sleep(800);
  return {{ ok:true, message:'已打开笔记 index=' + index + ' via=' + (el.className || el.tagName || '') }};
}})()"#
    )
}

const XHS_ACQ_WAIT_COMMENTS_JS: &str = r#"(async function(){
  function sleep(ms){ return new Promise(function(r){ setTimeout(r, ms); }); }
  var deadline = Date.now() + 12000;
  while (Date.now() < deadline) {
    var box = document.querySelector('.comments-container')
      || document.querySelector('#noteContainer .comments-container')
      || document.querySelector('.note-detail-mask .comments-container');
    if (box) {
      var n = box.querySelectorAll('.parent-comment').length;
      return { ok:true, count:n, message:'评论区就绪 parent-comment=' + n };
    }
    // 详情弹层出现但评论尚未挂载时也再等一会
    var detail = document.querySelector('.note-detail-mask')
      || document.querySelector('#noteContainer')
      || document.querySelector('.note-container');
    if (detail) {
      await sleep(400);
      continue;
    }
    await sleep(300);
  }
  return { ok:false, count:0, message:'等待 .comments-container 超时（请确认已点开 .note-item .cover.mask.ld）' };
})()"#;

fn xhs_acq_match_and_reply_js(
    keywords_json: &str,
    reply_json: &str,
    max_replies: i64,
    max_scrolls: i64,
) -> String {
    format!(
        r#"(async function(){{
  var keywords = {keywords_json};
  var replyText = {reply_json};
  var maxReplies = {max_replies};
  var maxScrolls = {max_scrolls};
  function sleep(ms){{ return new Promise(function(r){{ setTimeout(r, ms); }}); }}
  function dispatchKey(el, type, key, code, keyCode){{
    el.dispatchEvent(new KeyboardEvent(type, {{
      key:key, code:code, keyCode:keyCode, which:keyCode, bubbles:true, cancelable:true, composed:true
    }}));
  }}
  function norm(s){{ return String(s || '').toLowerCase(); }}
  function hitText(text){{
    var t = norm(text);
    for (var i=0;i<keywords.length;i++){{
      var k = norm(keywords[i]);
      if (k && t.indexOf(k) >= 0) return true;
    }}
    return false;
  }}
  function commentBox(){{
    return document.querySelector('.comments-container')
      || document.querySelector('#noteContainer .comments-container');
  }}
  function noteScroller(){{
    return document.querySelector('.note-content')
      || document.querySelector('.note-scroller')
      || document.querySelector('.comments-container')
      || document.querySelector('#noteContainer');
  }}
  function listParents(box){{
    return Array.prototype.slice.call(box.querySelectorAll('.parent-comment'));
  }}
  function fingerprint(pc, text){{
    var id = pc.getAttribute('id') || pc.getAttribute('data-id') || '';
    return id + '|' + text.slice(0, 80);
  }}
  async function fillAndSend(input){{
    try {{ input.focus(); input.click(); }} catch(e) {{}}
    await sleep(120);
    var isInput = input.tagName === 'TEXTAREA' || input.tagName === 'INPUT';
    if (isInput) {{
      input.value = replyText;
      input.dispatchEvent(new Event('input', {{ bubbles:true }}));
    }} else {{
      input.textContent = replyText;
      try {{
        input.dispatchEvent(new InputEvent('input', {{ bubbles:true, data:replyText, inputType:'insertText' }}));
      }} catch(e) {{
        input.dispatchEvent(new Event('input', {{ bubbles:true }}));
      }}
    }}
    await sleep(200);
    dispatchKey(input, 'keydown', 'Enter', 'Enter', 13);
    await sleep(500);
  }}
  async function processVisible(box, seen){{
    var parents = listParents(box);
    var matched = 0, sent = 0;
    var details = [];
    for (var i=0;i<parents.length;i++){{
      if (sent >= maxReplies) break;
      var pc = parents[i];
      var note = pc.querySelector('.note-text');
      var text = note ? (note.innerText || note.textContent || '') : (pc.innerText || '');
      text = String(text).replace(/\s+/g,' ').trim();
      if (!hitText(text)) continue;
      var fp = fingerprint(pc, text);
      if (seen[fp]) continue;
      seen[fp] = true;
      matched++;
    // 匹配后必须点 class="reds-icon reply-icon" 打开回复输入框
      var icon = pc.querySelector('.reds-icon.reply-icon')
        || pc.querySelector('svg.reds-icon.reply-icon')
        || pc.querySelector('.interactions .reds-icon.reply-icon')
        || pc.querySelector('.right .info .interactions .reds-icon.reply-icon');
      if (!icon) {{
        details.push({{ index:i, ok:false, detail:'无 .reds-icon.reply-icon' }});
        continue;
      }}
      try {{
        pc.scrollIntoView({{ block:'center', inline:'nearest' }});
        icon.scrollIntoView({{ block:'center', inline:'nearest' }});
      }} catch(e) {{}}
      await sleep(200);
      try {{
        var r = icon.getBoundingClientRect();
        var cx = r.left + r.width / 2;
        var cy = r.top + r.height / 2;
        var opts = {{ bubbles:true, cancelable:true, view:window, clientX:cx, clientY:cy, buttons:1 }};
        icon.dispatchEvent(new PointerEvent('pointerdown', opts));
        icon.dispatchEvent(new MouseEvent('mousedown', opts));
        icon.dispatchEvent(new PointerEvent('pointerup', opts));
        icon.dispatchEvent(new MouseEvent('mouseup', opts));
        icon.dispatchEvent(new MouseEvent('click', opts));
        try {{ icon.click(); }} catch(e2) {{}}
        // 若点在 svg 上未冒泡，再点父级带 reply-icon 的节点
        var wrap = icon.closest('.reply-icon') || icon.parentElement;
        if (wrap && wrap !== icon) {{
          try {{ wrap.click(); }} catch(e3) {{}}
        }}
      }} catch(e) {{
        details.push({{ index:i, ok:false, detail:'点 .reds-icon.reply-icon 失败' }});
        continue;
      }}
      // 等待输入框出现
      var input = null;
      for (var w=0; w<12; w++) {{
        await sleep(200);
        input = document.querySelector('.interactions.engage-bar .content-input')
          || document.querySelector('.engage-bar .content-input')
          || document.querySelector('.content-input')
          || document.querySelector('[contenteditable="true"].content-input')
          || document.querySelector('.engage-bar [contenteditable="true"]');
        if (input) break;
      }}
      if (!input) {{
        details.push({{ index:i, ok:false, detail:'已点 reply-icon 但未出现 .content-input' }});
        continue;
      }}
      await fillAndSend(input);
      sent++;
      details.push({{ index:i, ok:true, detail: text.slice(0, 40) }});
    }}
    return {{ matched:matched, sent:sent, details:details, total:parents.length }};
  }}
  async function scrollLoadMore(){{
    var scroller = noteScroller();
    if (!scroller) return {{ ok:false, reason:'no-scroller' }};
    var before = scroller.scrollTop || 0;
    var beforeH = scroller.scrollHeight || 0;
    var beforeCount = 0;
    var box = commentBox();
    if (box) beforeCount = listParents(box).length;
    try {{
      scroller.scrollTop = scroller.scrollHeight;
      scroller.dispatchEvent(new Event('scroll', {{ bubbles:true }}));
    }} catch(e) {{}}
    // 兜底：对 note-content 派发 wheel，触发无限加载
    try {{
      var rect = scroller.getBoundingClientRect();
      var cx = rect.left + rect.width / 2;
      var cy = rect.top + Math.min(rect.height - 20, rect.height * 0.85);
      scroller.dispatchEvent(new WheelEvent('wheel', {{
        bubbles:true, cancelable:true, deltaY: 800, clientX: cx, clientY: cy
      }}));
    }} catch(e) {{}}
    await sleep(900);
    var afterCount = beforeCount;
    box = commentBox();
    if (box) afterCount = listParents(box).length;
    var grew = afterCount > beforeCount || (scroller.scrollHeight || 0) > beforeH + 8;
    return {{
      ok:true,
      grew: !!grew,
      beforeCount: beforeCount,
      afterCount: afterCount,
      scrollTop: scroller.scrollTop || before
    }};
  }}

  var box = commentBox();
  if (!box) return {{ ok:false, matched:0, sent:0, scrolls:0, message:'无评论区' }};
  var seen = {{}};
  var matched = 0, sent = 0, scrolls = 0;
  var details = [];
  var round = await processVisible(box, seen);
  matched += round.matched;
  sent += round.sent;
  details = details.concat(round.details);

  while (sent < maxReplies && scrolls < maxScrolls) {{
    var load = await scrollLoadMore();
    scrolls++;
    if (!load.ok) break;
    box = commentBox();
    if (!box) break;
    round = await processVisible(box, seen);
    matched += round.matched;
    sent += round.sent;
    details = details.concat(round.details);
    // 连续下滑无新增评论时提前结束，换下一篇
    if (!load.grew && round.matched === 0) break;
  }}

  return {{
    ok:true,
    matched:matched,
    sent:sent,
    scrolls:scrolls,
    details:details,
    message:'匹配 ' + matched + ' / 发送 ' + sent + ' / 下滑 ' + scrolls + ' 次'
  }};
}})()"#
    )
}

/// 找下一条待回复的匹配评论，返回 `.reds-icon.reply-icon` 中心坐标（供 CDP 真点击）
fn xhs_find_reply_target_js(keywords_json: &str, seen_json: &str) -> String {
    format!(
        r#"(function(){{
  var keywords = {keywords_json};
  var seen = {seen_json};
  function norm(s){{ return String(s || '').toLowerCase(); }}
  function hitText(text){{
    var t = norm(text);
    for (var i=0;i<keywords.length;i++){{
      var k = norm(keywords[i]);
      if (k && t.indexOf(k) >= 0) return true;
    }}
    return false;
  }}
  var box = document.querySelector('.comments-container')
    || document.querySelector('#noteContainer .comments-container');
  if (!box) return {{ ok:false, message:'无评论区' }};
  var parents = Array.prototype.slice.call(box.querySelectorAll('.parent-comment'));
  for (var i=0;i<parents.length;i++){{
    var pc = parents[i];
    var note = pc.querySelector('.note-text');
    var text = note ? (note.innerText || note.textContent || '') : (pc.innerText || '');
    text = String(text).replace(/\s+/g,' ').trim();
    if (!hitText(text)) continue;
    var id = pc.getAttribute('id') || pc.getAttribute('data-id') || '';
    var fp = id + '|' + text.slice(0, 80);
    if (seen[fp]) continue;
    var icon = pc.querySelector('.reds-icon.reply-icon')
      || pc.querySelector('svg.reds-icon.reply-icon')
      || pc.querySelector('.interactions .reds-icon.reply-icon');
    if (!icon) continue;
    try {{
      pc.scrollIntoView({{ block:'center', inline:'nearest' }});
      icon.scrollIntoView({{ block:'center', inline:'nearest' }});
    }} catch(e) {{}}
    var r = icon.getBoundingClientRect();
    if (r.width < 2 || r.height < 2) continue;
    var author = '';
    var authorEl = pc.querySelector('.author-wrapper .name')
      || pc.querySelector('.author .name')
      || pc.querySelector('.name')
      || pc.querySelector('a.name')
      || pc.querySelector('[class*="author"] .name');
    if (authorEl) author = String(authorEl.innerText || authorEl.textContent || '').replace(/\\s+/g,' ').trim();
    var matchedKw = '';
    var tLow = norm(text);
    for (var ki=0;ki<keywords.length;ki++){{
      var kk = norm(keywords[ki]);
      if (kk && tLow.indexOf(kk) >= 0) {{ matchedKw = String(keywords[ki] || ''); break; }}
    }}
    return {{
      ok:true,
      index:i,
      fp:fp,
      text:text.slice(0, 200),
      author: author.slice(0, 40),
      matchedKeyword: matchedKw,
      x: r.left + r.width / 2,
      y: r.top + r.height / 2,
      w: r.width,
      h: r.height,
      message:'命中评论 #' + i + '，定位 .reds-icon.reply-icon'
    }};
  }}
  return {{ ok:false, total:parents.length, message:'当前可视区无待回复匹配评论' }};
}})()"#
    )
}

const XHS_SCROLL_COMMENTS_ONCE_JS: &str = r#"(function(){
  var el = document.querySelector('.note-content')
    || document.querySelector('.comments-container')
    || document.querySelector('#noteContainer');
  if (!el) return { ok:false };
  var before = el.scrollTop || 0;
  var beforeH = el.scrollHeight || 0;
  var beforeCount = 0;
  var box = document.querySelector('.comments-container');
  if (box) beforeCount = box.querySelectorAll('.parent-comment').length;
  el.scrollTop = (el.scrollTop || 0) + Math.max(320, el.clientHeight * 0.85);
  el.dispatchEvent(new Event('scroll', { bubbles:true }));
  var afterCount = box ? box.querySelectorAll('.parent-comment').length : beforeCount;
  return {
    ok:true,
    grew: afterCount > beforeCount || (el.scrollHeight || 0) > beforeH + 8,
    scrollTop: el.scrollTop || before
  };
})()"#;

fn xhs_fill_and_send_reply_js(reply_json: &str) -> String {
    format!(
        r#"(async function(){{
  var replyText = {reply_json};
  function sleep(ms){{ return new Promise(function(r){{ setTimeout(r, ms); }}); }}
  function rand(a,b){{ return a + Math.floor(Math.random() * (b - a + 1)); }}
  function dispatchKey(el, type, key, code, keyCode){{
    el.dispatchEvent(new KeyboardEvent(type, {{
      key:key, code:code, keyCode:keyCode, which:keyCode, bubbles:true, cancelable:true, composed:true
    }}));
  }}
  var input = null;
  for (var w=0; w<18; w++){{
    input = document.querySelector('.interactions.engage-bar .content-input')
      || document.querySelector('.engage-bar .content-input')
      || document.querySelector('.content-input')
      || document.querySelector('[contenteditable="true"].content-input')
      || document.querySelector('.engage-bar [contenteditable="true"]');
    if (input) break;
    await sleep(rand(180, 320));
  }}
  if (!input) return {{ ok:false, message:'已点 reply-icon 但未出现 .content-input' }};
  try {{ input.focus(); input.click(); }} catch(e) {{}}
  await sleep(rand(280, 520)); // 拟人：点开后先看一眼再输入
  var isInput = input.tagName === 'TEXTAREA' || input.tagName === 'INPUT';
  var chars = Array.from(String(replyText));
  if (isInput) {{
    input.value = '';
    input.dispatchEvent(new Event('input', {{ bubbles:true }}));
  }} else {{
    input.textContent = '';
    input.dispatchEvent(new Event('input', {{ bubbles:true }}));
  }}
  // 逐字输入，偶尔短停顿
  var buf = '';
  for (var i=0;i<chars.length;i++){{
    buf += chars[i];
    if (isInput) {{
      input.value = buf;
      input.dispatchEvent(new Event('input', {{ bubbles:true }}));
    }} else {{
      input.textContent = buf;
      try {{
        input.dispatchEvent(new InputEvent('input', {{ bubbles:true, data:chars[i], inputType:'insertText' }}));
      }} catch(e) {{
        input.dispatchEvent(new Event('input', {{ bubbles:true }}));
      }}
    }}
    await sleep(rand(35, 95));
    if (i > 0 && i % rand(6, 11) === 0) await sleep(rand(120, 260));
  }}
  await sleep(rand(450, 900)); // 输入完再停顿再回车
  dispatchKey(input, 'keydown', 'Enter', 'Enter', 13);
  await sleep(rand(40, 90));
  dispatchKey(input, 'keyup', 'Enter', 'Enter', 13);
  await sleep(rand(600, 1100));
  return {{ ok:true, message:'已拟人填入自定义回复并回车发送' }};
}})()"#
    )
}

/// 匹配评论 → CDP 点击 `.reds-icon.reply-icon` → 填入回复并发送
struct AcqReplyCtx {
    profile_id: String,
    search_keyword: String,
    reply_text: String,
    note_title: String,
    note_index: i64,
}

fn xhs_acq_do_match_and_reply(
    app: Option<&AppHandle>,
    ws: &str,
    keywords_json: &str,
    reply_json: &str,
    max_replies: i64,
    max_scrolls: i64,
    mut steps: Option<&mut Vec<XhsUnreadStep>>,
    history_ctx: Option<&AcqReplyCtx>,
) -> Result<(i64, i64, i64, String), String> {
    let mut matched = 0i64;
    let mut sent = 0i64;
    let mut scrolls = 0i64;
    let mut seen: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    let mut idle_scrolls = 0i64;

    let mut push_local = |name: String, ok: bool, detail: String| {
        if let Some(s) = steps.as_deref_mut() {
            push_step(s, &name, ok, detail.clone());
        }
        acq_emit(app, &name, ok, &detail);
    };

    while sent < max_replies {
        let seen_json = serde_json::Value::Object(seen.clone()).to_string();
        let find_js = xhs_find_reply_target_js(keywords_json, &seen_json);
        let target = cdp_runtime_evaluate(ws, &find_js, 12)?;
        if json_bool(&target, "ok") {
            matched += 1;
            let fp = json_str(&target, "fp");
            let comment_text = json_str(&target, "text");
            let author_name = json_str(&target, "author");
            let matched_kw = json_str(&target, "matchedKeyword");
            let x = target.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = target.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            push_local(
                format!("find_reply_{matched}"),
                true,
                format!(
                    "{} @({:.0},{:.0})",
                    json_str(&target, "message"),
                    x,
                    y
                ),
            );
            // 拟人：先扫一眼评论，再移过去点 .reds-icon.reply-icon
            human_sleep(350, 780);
            let click_ok = cdp_mouse_click_human(ws, x, y).is_ok();
            push_local(
                format!("click_reply_icon_{matched}"),
                click_ok,
                format!("拟人点击 .reds-icon.reply-icon ({x:.0},{y:.0})"),
            );
            // JS 兜底轻点（仅在 CDP 后输入框仍未出现时由 fill 路径等待）
            let _ = cdp_runtime_evaluate(
                ws,
                &format!(
                    r#"(function(){{
  var box = document.querySelector('.comments-container');
  if (!box) return {{ ok:false }};
  var parents = box.querySelectorAll('.parent-comment');
  var idx = {};
  var pc = parents[idx];
  if (!pc) return {{ ok:false }};
  var icon = pc.querySelector('.reds-icon.reply-icon') || pc.querySelector('svg.reds-icon.reply-icon');
  if (!icon) return {{ ok:false }};
  try {{ icon.click(); }} catch(e) {{}}
  return {{ ok:true }};
}})()"#,
                    target.get("index").and_then(|v| v.as_i64()).unwrap_or(0)
                ),
                8,
            );
            human_sleep(520, 980); // 等输入框动画
            let filled = cdp_runtime_evaluate(ws, &xhs_fill_and_send_reply_js(reply_json), 90)?;
            let fok = json_bool(&filled, "ok");
            push_local(
                format!("fill_send_{matched}"),
                fok,
                json_str(&filled, "message"),
            );
            if !fp.is_empty() {
                seen.insert(fp, serde_json::json!(true));
            }
            if fok {
                sent += 1;
                idle_scrolls = 0;
            }
            if let (Some(app_h), Some(ctx)) = (app, history_ctx) {
                if let Some(db) = app_h.try_state::<crate::library::LibraryState>() {
                    crate::knowledge::acq_reply_history_insert_quiet(
                        db.inner(),
                        crate::knowledge::AcqReplyHistoryInsert {
                            profile_id: ctx.profile_id.clone(),
                            search_keyword: ctx.search_keyword.clone(),
                            comment_keyword: if matched_kw.is_empty() {
                                comment_text.chars().take(40).collect()
                            } else {
                                matched_kw
                            },
                            author_name,
                            comment_text,
                            reply_text: ctx.reply_text.clone(),
                            note_title: ctx.note_title.clone(),
                            note_index: ctx.note_index,
                            ok: fok,
                        },
                    );
                }
            }
            human_sleep(800, 1600); // 回复后停顿再找下一条
            continue;
        }

        // 无可点匹配 → 拟人下滑加载更多
        if scrolls >= max_scrolls {
            break;
        }
        human_sleep(400, 900);
        if let Ok(loc) = cdp_runtime_evaluate(ws, XHS_LOCATE_COMMENT_SCROLLER_JS, 8) {
            if json_bool(&loc, "scrollerFound") {
                let sx = loc.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let sy = loc.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let _ = cdp_mouse_wheel_human(ws, sx, sy, human_jitter(560.0, 120.0));
            }
        }
        let load = cdp_runtime_evaluate(ws, XHS_SCROLL_COMMENTS_ONCE_JS, 8)?;
        scrolls += 1;
        push_local(
            format!("scroll_{scrolls}"),
            json_bool(&load, "ok"),
            format!("拟人下滑加载 #{scrolls}"),
        );
        human_sleep(700, 1400);
        if !json_bool(&load, "grew") {
            idle_scrolls += 1;
            if idle_scrolls >= 2 {
                break;
            }
        } else {
            idle_scrolls = 0;
        }
    }

    let message = format!("匹配 {matched} / 发送 {sent} / 下滑 {scrolls} 次");
    Ok((matched, sent, scrolls, message))
}

const XHS_ACQ_CLOSE_NOTE_JS: &str = r#"(async function(){
  function sleep(ms){ return new Promise(function(r){ setTimeout(r, ms); }); }
  function dispatchKey(el, type){
    el.dispatchEvent(new KeyboardEvent(type, {
      key:'Escape', code:'Escape', keyCode:27, which:27, bubbles:true, cancelable:true
    }));
  }
  var closeBtn = document.querySelector('.close-circle')
    || document.querySelector('[class*="close-box"]')
    || document.querySelector('.note-detail .close')
    || document.querySelector('div.close-box')
    || document.querySelector('[aria-label="关闭"]');
  if (closeBtn) {
    try { closeBtn.click(); await sleep(400); return { ok:true, message:'已点关闭' }; } catch(e) {}
  }
  try {
    dispatchKey(document.body, 'keydown');
    await sleep(350);
  } catch(e) {}
  var still = !!document.querySelector('.comments-container');
  return { ok: !still, message: still ? '关闭详情可能失败' : '已 Esc 关闭' };
})()"#;

/// 获客任务期间保持顶部「AI正在操作浏览器」横幅（页面跳转后需重注）
fn acq_ensure_ai_banner(debug_port: u16, ws: &str) {
    let _ = cdp_set_ai_banner_on_xhs_pages(debug_port, true);
    let _ = cdp_set_ai_banner(ws, true);
}

fn acq_clear_ai_banner(debug_port: u16, ws: &str) {
    let _ = cdp_set_ai_banner_on_xhs_pages(debug_port, false);
    let _ = cdp_set_ai_banner(ws, false);
}

fn xhs_acq_run_job(
    app: &AppHandle,
    profile_id: String,
    tab_id: String,
    ws: String,
    debug_port: u16,
    search_kw: String,
    comment_kw_display: String,
    match_keywords: Vec<String>,
    reply_text: String,
    sort_label: String,
    max_notes: usize,
    max_replies: i64,
    max_scrolls: i64,
) -> XhsAcqRunResult {
    let probed_at = now_unix_secs();
    let mut steps = Vec::new();
    let app_ref = Some(app);

    acq_ensure_ai_banner(debug_port, &ws);
    acq_push(
        app_ref,
        &mut steps,
        "ai_banner",
        true,
        "已显示「AI正在操作浏览器」横幅",
    );

    let fail = |steps: Vec<XhsUnreadStep>, message: String| XhsAcqRunResult {
        ok: false,
        profile_id: profile_id.clone(),
        tab_id: tab_id.clone(),
        keyword: search_kw.clone(),
        comment_keyword: comment_kw_display.clone(),
        sort_label: sort_label.clone(),
        notes_opened: 0,
        comments_matched: 0,
        replies_sent: 0,
        steps,
        message,
        probed_at,
    };

    // 1) 搜索（仅用笔记搜索词）
    let kw_json = serde_json::to_string(&search_kw).unwrap_or_else(|_| "\"\"".into());
    let search = match cdp_runtime_evaluate(&ws, &xhs_acq_search_js(&kw_json), 20) {
        Ok(v) => v,
        Err(e) => {
            acq_push(app_ref, &mut steps, "search", false, &e);
            return fail(steps, e);
        }
    };
    let search_ok = json_bool(&search, "ok");
    acq_push(app_ref, &mut steps, "search", search_ok, json_str(&search, "message"));
    if !search_ok {
        return fail(steps, json_str(&search, "message"));
    }
    // 搜索会跳转结果页，重新注入横幅
    acq_ensure_ai_banner(debug_port, &ws);
    human_sleep(1600, 2600); // 搜索后等人看结果

    // 2) 排序
    let label_json = serde_json::to_string(&sort_label).unwrap_or_else(|_| "\"最新\"".into());
    // CDP hover 辅助（拟人移入）
    if let Ok(pre) = cdp_runtime_evaluate(
        &ws,
        r#"(function(){
          var f = document.querySelector('.filter.ai-chat-filter') || document.querySelector('.ai-chat-filter');
          if (!f) return { ok:false };
          var r = f.getBoundingClientRect();
          return { ok:true, x: r.left + r.width/2, y: r.top + r.height/2 };
        })()"#,
        8,
    ) {
        if json_bool(&pre, "ok") {
            let x = pre.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = pre.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let _ = cdp_mouse_move_human(&ws, x, y);
            human_sleep(280, 520);
        }
    }
    match cdp_runtime_evaluate(&ws, &xhs_acq_click_sort_js(&label_json), 15) {
        Ok(sort) => {
            let sort_ok = json_bool(&sort, "ok");
            acq_push(app_ref, &mut steps, "sort", sort_ok, json_str(&sort, "message"));
            if !sort_ok {
                acq_push(app_ref, &mut steps, "sort_warn", false, "排序失败，继续处理笔记");
            }
        }
        Err(e) => {
            acq_push(app_ref, &mut steps, "sort", false, &e);
            acq_push(app_ref, &mut steps, "sort_warn", false, "排序失败，继续处理笔记");
        }
    }
    human_sleep(1100, 1900);

    // 3) 遍历笔记
    let list = match cdp_runtime_evaluate(&ws, XHS_ACQ_LIST_NOTES_JS, 10) {
        Ok(v) => v,
        Err(e) => {
            acq_push(app_ref, &mut steps, "list_notes", false, &e);
            return fail(steps, e);
        }
    };
    let note_count = list
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    acq_push(
        app_ref,
        &mut steps,
        "list_notes",
        json_bool(&list, "ok"),
        json_str(&list, "message"),
    );
    let limit = note_count.min(max_notes);
    let mut notes_opened = 0i64;
    let mut comments_matched = 0i64;
    let mut replies_sent = 0i64;
    let kw_arr = serde_json::to_string(&match_keywords).unwrap_or_else(|_| "[]".into());
    let reply_json = serde_json::to_string(&reply_text).unwrap_or_else(|_| "\"\"".into());

    for i in 0..limit {
        human_sleep(600, 1400); // 打开下一篇前停顿
        let opened = match cdp_runtime_evaluate(&ws, &xhs_acq_click_note_js(i), 12) {
            Ok(v) => v,
            Err(e) => {
                acq_push(app_ref, &mut steps, &format!("open_note_{i}"), false, &e);
                continue;
            }
        };
        let open_ok = json_bool(&opened, "ok");
        acq_push(
            app_ref,
            &mut steps,
            &format!("open_note_{i}"),
            open_ok,
            json_str(&opened, "message"),
        );
        if !open_ok {
            continue;
        }
        notes_opened += 1;
        human_sleep(500, 1000); // 等人看详情加载
        let wait = match cdp_runtime_evaluate(&ws, XHS_ACQ_WAIT_COMMENTS_JS, 18) {
            Ok(v) => v,
            Err(e) => {
                acq_push(app_ref, &mut steps, &format!("wait_comments_{i}"), false, &e);
                let _ = cdp_runtime_evaluate(&ws, XHS_ACQ_CLOSE_NOTE_JS, 10);
                continue;
            }
        };
        acq_push(
            app_ref,
            &mut steps,
            &format!("wait_comments_{i}"),
            json_bool(&wait, "ok"),
            json_str(&wait, "message"),
        );
        if !json_bool(&wait, "ok") {
            let _ = cdp_runtime_evaluate(&ws, XHS_ACQ_CLOSE_NOTE_JS, 10);
            continue;
        }
        // 打开笔记后 DOM 可能重建，保持横幅可见
        acq_ensure_ai_banner(debug_port, &ws);
        human_sleep(400, 900);
        let note_title = cdp_runtime_evaluate(
            &ws,
            r#"(function(){
  var el = document.querySelector('#detail-title')
    || document.querySelector('.note-content .title')
    || document.querySelector('[class*="title"] span')
    || document.querySelector('h1');
  var t = el ? String(el.innerText || el.textContent || '').replace(/\s+/g,' ').trim() : '';
  return { ok:true, title: t.slice(0, 80) };
})()"#,
            6,
        )
        .map(|v| json_str(&v, "title"))
        .unwrap_or_default();
        let hist = AcqReplyCtx {
            profile_id: profile_id.clone(),
            search_keyword: search_kw.clone(),
            reply_text: reply_text.clone(),
            note_title,
            note_index: i as i64,
        };
        // 匹配评论 → 拟人点击 .reds-icon.reply-icon → 逐字填文案发送
        match xhs_acq_do_match_and_reply(
            app_ref,
            &ws,
            &kw_arr,
            &reply_json,
            max_replies,
            max_scrolls,
            Some(&mut steps),
            Some(&hist),
        ) {
            Ok((matched, sent, scrolled, msg)) => {
                comments_matched += matched;
                replies_sent += sent;
                acq_push(
                    app_ref,
                    &mut steps,
                    &format!("reply_note_{i}"),
                    sent > 0 || matched > 0,
                    format!("{msg}（下滑 {scrolled}）"),
                );
            }
            Err(e) => {
                acq_push(app_ref, &mut steps, &format!("reply_note_{i}"), false, &e);
            }
        }
        human_sleep(500, 1100);
        match cdp_runtime_evaluate(&ws, XHS_ACQ_CLOSE_NOTE_JS, 10) {
            Ok(closed) => {
                acq_push(
                    app_ref,
                    &mut steps,
                    &format!("close_note_{i}"),
                    json_bool(&closed, "ok"),
                    json_str(&closed, "message"),
                );
            }
            Err(e) => {
                acq_push(app_ref, &mut steps, &format!("close_note_{i}"), false, &e);
            }
        }
        human_sleep(900, 1800); // 换下一篇前多停一会
    }

    let ok = notes_opened > 0;
    let message = format!(
        "搜索「{search_kw}」评论匹配「{comment_kw_display}」排序「{sort_label}」：打开 {notes_opened} 篇，匹配评论 {comments_matched}，发送 {replies_sent}"
    );
    XhsAcqRunResult {
        ok,
        profile_id,
        tab_id,
        keyword: search_kw,
        comment_keyword: comment_kw_display,
        sort_label,
        notes_opened,
        comments_matched,
        replies_sent,
        steps,
        message,
        probed_at,
    }
}

/// 前端卡在「执行中」时强制清除后台占用标记（不杀线程；线程若仍在跑会自行结束）
#[tauri::command]
pub fn xhs_acq_force_idle() -> Result<serde_json::Value, String> {
    let was = ACQ_JOB_RUNNING.swap(false, Ordering::SeqCst);
    Ok(serde_json::json!({
        "ok": true,
        "wasRunning": was,
        "message": if was {
            "已清除获客占用标记，可重新开始"
        } else {
            "获客未在占用中"
        }
    }))
}

/// 立即返回，CDP 获客在独立线程执行；进度 `xhs-acq-progress`，结束 `xhs-acq-finished`
#[tauri::command]
pub fn xhs_acq_run(
    app: AppHandle,
    db: State<'_, crate::library::LibraryState>,
    body: XhsAcqRunBody,
) -> Result<XhsAcqStartResult, String> {
    let search_kw = body.keyword.trim().to_string();
    if search_kw.is_empty() {
        return Err("请填写笔记搜索关键词".into());
    }
    let comment_kw_raw = body
        .comment_keyword
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(search_kw.as_str())
        .to_string();
    if comment_kw_raw.trim().is_empty() {
        return Err("请填写评论匹配关键词".into());
    }
    let reply_text = body.reply_text.trim().to_string();
    if reply_text.is_empty() {
        return Err("请填写回复文案".into());
    }
    let sort_label = body
        .sort_label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("最新")
        .to_string();
    let max_notes = body.max_notes.unwrap_or(5).clamp(1, 30) as usize;
    let max_replies = body.max_replies_per_note.unwrap_or(1).clamp(1, 10);
    let max_scrolls = body.max_comment_scrolls.unwrap_or(5).clamp(0, 10);

    let match_keywords = parse_comment_keywords(&comment_kw_raw);
    if match_keywords.is_empty() {
        return Err("请填写评论匹配关键词".into());
    }
    let comment_kw_display = match_keywords.join("@");

    if ACQ_JOB_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("获客任务已在后台运行，请等待本轮结束".into());
    }

    let (profile, target, ws) = match resolve_xhs_cdp_ws(&db, body.profile_id) {
        Ok(v) => v,
        Err(e) => {
            ACQ_JOB_RUNNING.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    let profile_id = profile.id.clone();
    let tab_id = target.id.clone();
    let debug_port = profile.debug_port;

    acq_emit(
        Some(&app),
        "job_start",
        true,
        "获客任务已在后台启动，可继续操作主界面",
    );

    let app_bg = app.clone();
    let profile_id_bg = profile_id.clone();
    let tab_id_bg = tab_id.clone();
    let profile_id_err = profile_id.clone();
    let tab_id_err = tab_id.clone();
    thread::spawn(move || {
        // 横幅放进 catch_unwind：避免 CDP 卡住时连 finished 都发不出去
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            acq_ensure_ai_banner(debug_port, &ws);
            let r = xhs_acq_run_job(
                &app_bg,
                profile_id_bg,
                tab_id_bg,
                ws.clone(),
                debug_port,
                search_kw,
                comment_kw_display,
                match_keywords,
                reply_text,
                sort_label,
                max_notes,
                max_replies,
                max_scrolls,
            );
            acq_clear_ai_banner(debug_port, &ws);
            r
        }));
        // 兜底清横幅（panic / 提前 return 路径）
        acq_clear_ai_banner(debug_port, &ws);
        ACQ_JOB_RUNNING.store(false, Ordering::SeqCst);
        match result {
            Ok(r) => {
                let _ = app_bg.emit("xhs-acq-finished", r);
            }
            Err(_) => {
                let _ = app_bg.emit(
                    "xhs-acq-finished",
                    XhsAcqRunResult {
                        ok: false,
                        profile_id: profile_id_err,
                        tab_id: tab_id_err,
                        keyword: String::new(),
                        comment_keyword: String::new(),
                        sort_label: String::new(),
                        notes_opened: 0,
                        comments_matched: 0,
                        replies_sent: 0,
                        steps: vec![],
                        message: "获客任务异常中断".into(),
                        probed_at: now_unix_secs(),
                    },
                );
            }
        }
    });

    Ok(XhsAcqStartResult {
        ok: true,
        profile_id,
        tab_id,
        message: "获客已在后台启动".into(),
    })
}

pub fn kill_all_browsers(browser: &BrowserProcess) {
    if let Ok(mut guard) = browser.0.lock() {
        for (_, mut child) in guard.drain() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
