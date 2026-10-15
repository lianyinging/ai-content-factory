use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use tauri::{AppHandle, Manager};

pub const RELEASES_BASE: &str =
    "https://team-swarm-oss.oss-cn-shanghai.aliyuncs.com/ai_content/releases/";
pub const LATEST_JSON_URL: &str =
    "https://team-swarm-oss.oss-cn-shanghai.aliyuncs.com/ai_content/releases/latest.json";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub app_name: String,
    pub version: String,
    pub identifier: String,
    pub tauri_version: String,
    pub os: String,
    pub os_family: String,
    pub arch: String,
    pub app_data_dir: String,
    pub resource_dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub notes: String,
    pub published_at: String,
    pub download_url: String,
    pub page_url: String,
    pub platform_key: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct LatestManifest {
    version: String,
    #[serde(default)]
    notes: String,
    #[serde(default, alias = "published_at", alias = "publishedAt")]
    published_at: String,
    #[serde(default)]
    downloads: Value,
    #[serde(default, alias = "page_url", alias = "pageUrl")]
    page_url: String,
    #[serde(default)]
    macos: String,
    #[serde(default)]
    windows: String,
    #[serde(default)]
    url: String,
}

fn parse_ver(s: &str) -> Vec<u64> {
    s.trim()
        .trim_start_matches('v')
        .trim_start_matches('V')
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse::<u64>().ok())
        .collect()
}

fn cmp_version(a: &str, b: &str) -> Ordering {
    let aa = parse_ver(a);
    let bb = parse_ver(b);
    let n = aa.len().max(bb.len());
    for i in 0..n {
        let x = *aa.get(i).unwrap_or(&0);
        let y = *bb.get(i).unwrap_or(&0);
        match x.cmp(&y) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

fn platform_key() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("macos", "aarch64") => "darwin-aarch64".into(),
        ("macos", "x86_64") => "darwin-x86_64".into(),
        ("windows", "x86_64") => "windows-x86_64".into(),
        ("windows", "aarch64") => "windows-aarch64".into(),
        ("linux", "x86_64") => "linux-x86_64".into(),
        ("linux", "aarch64") => "linux-aarch64".into(),
        _ => format!("{os}-{arch}"),
    }
}

fn pick_download(manifest: &LatestManifest, key: &str) -> String {
    if let Some(obj) = manifest.downloads.as_object() {
        for k in [
            key,
            // 兼容简写
            if key.starts_with("darwin") {
                "macos"
            } else if key.starts_with("windows") {
                "windows"
            } else {
                ""
            },
            if key.contains("aarch64") && key.starts_with("darwin") {
                "macos-arm64"
            } else if key.contains("x86_64") && key.starts_with("darwin") {
                "macos-x64"
            } else if key.starts_with("windows") {
                "win64"
            } else {
                ""
            },
        ] {
            if k.is_empty() {
                continue;
            }
            if let Some(v) = obj.get(k).and_then(|x| x.as_str()) {
                let u = v.trim();
                if !u.is_empty() {
                    return u.to_string();
                }
            }
        }
    }
    if key.starts_with("darwin") && !manifest.macos.trim().is_empty() {
        return manifest.macos.trim().to_string();
    }
    if key.starts_with("windows") && !manifest.windows.trim().is_empty() {
        return manifest.windows.trim().to_string();
    }
    if !manifest.url.trim().is_empty() {
        return manifest.url.trim().to_string();
    }
    String::new()
}

#[tauri::command]
pub fn get_system_info(app: AppHandle) -> Result<SystemInfo, String> {
    let pkg = app.package_info();
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "—".into());
    let resource_dir = app
        .path()
        .resource_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "—".into());

    Ok(SystemInfo {
        app_name: pkg.name.clone(),
        version: pkg.version.to_string(),
        identifier: app.config().identifier.clone(),
        tauri_version: tauri::VERSION.to_string(),
        os: std::env::consts::OS.to_string(),
        os_family: std::env::consts::FAMILY.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        app_data_dir,
        resource_dir,
    })
}

#[tauri::command]
pub fn check_app_update(app: AppHandle) -> Result<UpdateCheckResult, String> {
    let current = app.package_info().version.to_string();
    let platform = platform_key();

    let body = ureq::get(LATEST_JSON_URL)
        .set("User-Agent", "ai-content-factory-updater")
        .set("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| {
            format!(
                "无法读取更新信息（请确认已上传 latest.json）：{e}\n{LATEST_JSON_URL}"
            )
        })?
        .into_string()
        .map_err(|e| format!("读取更新响应失败: {e}"))?;

    let manifest: LatestManifest = serde_json::from_str(&body).map_err(|e| {
        format!("latest.json 解析失败: {e}\n请按 resources/releases/latest.json.example 格式上传")
    })?;

    let latest = manifest.version.trim().to_string();
    if latest.is_empty() {
        return Err("latest.json 缺少 version 字段".into());
    }

    let has_update = matches!(cmp_version(&latest, &current), Ordering::Greater);
    let mut download_url = pick_download(&manifest, &platform);
    let page_url = if !manifest.page_url.trim().is_empty() {
        manifest.page_url.trim().to_string()
    } else {
        RELEASES_BASE.to_string()
    };
    if download_url.is_empty() {
        download_url = page_url.clone();
    }

    let message = if has_update {
        format!("发现新版本 {latest}（当前 {current}）")
    } else {
        format!("已是最新版本（{current}）")
    };

    Ok(UpdateCheckResult {
        current_version: current,
        latest_version: latest,
        has_update,
        notes: manifest.notes.trim().to_string(),
        published_at: manifest.published_at.trim().to_string(),
        download_url,
        page_url,
        platform_key: platform,
        message,
    })
}
