//! Download remote note images into app data for offline library / competitor covers.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

pub fn media_cache_root(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?
        .join("note-media");
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir note-media: {e}"))?;
    Ok(dir)
}

fn safe_key(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "item".into()
    } else {
        s.chars().take(80).collect()
    }
}

fn ext_from_mime_or_url(mime: &str, url: &str) -> &'static str {
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
    let lower = url.to_ascii_lowercase();
    if lower.contains(".webp") {
        "webp"
    } else if lower.contains(".gif") {
        "gif"
    } else if lower.contains(".png") {
        "png"
    } else {
        "jpg"
    }
}

/// Download one image URL to `dest` (parent dirs created). Returns mime.
pub fn download_image_url(url: &str, dest: &Path) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("图片 URL 为空".into());
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("不支持的图片地址: {url}"));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout_read(std::time::Duration::from_secs(60))
        .build();
    let referer = if url.contains("xhscdn") || url.contains("xiaohongshu") {
        "https://www.xiaohongshu.com/"
    } else if url.contains("douyin") || url.contains("byteimg") || url.contains("tiktok") {
        "https://www.douyin.com/"
    } else {
        "https://www.xiaohongshu.com/"
    };
    let res = agent
        .get(url)
        .set(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
        )
        .set("Referer", referer)
        .set("Accept", "image/avif,image/webp,image/apng,image/*,*/*;q=0.8")
        .call()
        .map_err(|e| format!("下载图片失败: {e}"))?;
    let mime = res
        .header("content-type")
        .unwrap_or("image/jpeg")
        .split(';')
        .next()
        .unwrap_or("image/jpeg")
        .trim()
        .to_string();
    let mut bytes = Vec::new();
    res.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("读取图片失败: {e}"))?;
    if bytes.is_empty() {
        return Err("下载图片为空".into());
    }
    fs::write(dest, &bytes).map_err(|e| format!("写入图片失败: {e}"))?;
    Ok(mime)
}

#[derive(Debug, Clone, Default)]
pub struct CachedNoteMedia {
    pub cover_local: String,
    pub images_local: Vec<String>,
}

/// Cache cover + gallery under `note-media/{namespace}/{key}/`.
/// Failures on individual images are skipped; returns whatever succeeded.
pub fn cache_note_media(
    app: &AppHandle,
    namespace: &str,
    key: &str,
    cover_url: &str,
    image_urls: &[String],
) -> CachedNoteMedia {
    let mut out = CachedNoteMedia::default();
    let root = match media_cache_root(app) {
        Ok(r) => r,
        Err(_) => return out,
    };
    let dir = root.join(safe_key(namespace)).join(safe_key(key));
    let _ = fs::create_dir_all(&dir);

    let mut urls: Vec<String> = Vec::new();
    let cover = cover_url.trim();
    if !cover.is_empty() {
        urls.push(cover.to_string());
    }
    for u in image_urls {
        let t = u.trim();
        if t.is_empty() {
            continue;
        }
        if !urls.iter().any(|x| x == t) {
            urls.push(t.to_string());
        }
    }

    for (i, url) in urls.iter().enumerate() {
        // probe extension via temp name then rename — download to .tmp first
        let tmp = dir.join(format!("img_{i}.tmp"));
        match download_image_url(url, &tmp) {
            Ok(mime) => {
                let ext = ext_from_mime_or_url(&mime, url);
                let name = if i == 0 && !cover.is_empty() && url == cover {
                    format!("cover.{ext}")
                } else {
                    format!("img_{i}.{ext}")
                };
                let final_path = dir.join(&name);
                let _ = fs::remove_file(&final_path);
                if fs::rename(&tmp, &final_path).is_err() {
                    let _ = fs::copy(&tmp, &final_path);
                    let _ = fs::remove_file(&tmp);
                }
                let abs = final_path.to_string_lossy().to_string();
                if i == 0 && out.cover_local.is_empty() {
                    out.cover_local = abs.clone();
                }
                out.images_local.push(abs);
            }
            Err(_) => {
                let _ = fs::remove_file(&tmp);
            }
        }
    }
    out
}

pub fn remove_note_media(app: &AppHandle, namespace: &str, key: &str) {
    if let Ok(root) = media_cache_root(app) {
        let dir = root.join(safe_key(namespace)).join(safe_key(key));
        let _ = fs::remove_dir_all(dir);
    }
}
