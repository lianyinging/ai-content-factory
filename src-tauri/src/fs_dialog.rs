use std::path::PathBuf;

const MAX_TEXT_BYTES: u64 = 8 * 1024 * 1024;

/// 读取本地文本文件（供拖放导入等场景）。
#[tauri::command]
pub fn read_text_file(path: String) -> Result<String, String> {
    let p = PathBuf::from(path.trim());
    if !p.is_file() {
        return Err("不是有效文件".into());
    }
    let meta = std::fs::metadata(&p).map_err(|e| format!("无法读取文件: {e}"))?;
    if meta.len() > MAX_TEXT_BYTES {
        return Err("文件过大（上限 8MB）".into());
    }
    std::fs::read_to_string(&p).map_err(|e| format!("读取失败: {e}"))
}

/// 弹出系统「另存为」对话框，将文本写入用户选择的路径。
/// 取消则返回 `Ok(None)`。
#[tauri::command]
pub async fn save_text_with_dialog(
    default_name: String,
    content: String,
    filter_name: String,
    extensions: Vec<String>,
) -> Result<Option<String>, String> {
    let fname = default_name.trim().to_string();
    if fname.is_empty() {
        return Err("文件名不能为空".into());
    }
    let filter = if filter_name.trim().is_empty() {
        "文件".into()
    } else {
        filter_name
    };
    let exts = if extensions.is_empty() {
        vec!["txt".into()]
    } else {
        extensions
            .into_iter()
            .map(|e| e.trim().trim_start_matches('.').to_string())
            .filter(|e| !e.is_empty())
            .collect::<Vec<_>>()
    };
    if exts.is_empty() {
        return Err("扩展名无效".into());
    }

    let primary_ext = exts[0].clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let ext_refs: Vec<&str> = exts.iter().map(|s| s.as_str()).collect();
        rfd::FileDialog::new()
            .set_file_name(&fname)
            .add_filter(&filter, &ext_refs)
            .save_file()
    })
    .await
    .map_err(|e| format!("打开保存对话框失败: {e}"))?;

    let Some(path) = picked else {
        return Ok(None);
    };
    let path = ensure_extension(path, &primary_ext);
    std::fs::write(&path, content.as_bytes()).map_err(|e| format!("写入失败: {e}"))?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

fn ensure_extension(path: PathBuf, ext: &str) -> PathBuf {
    let ext = ext.trim_start_matches('.');
    if ext.is_empty() {
        return path;
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some(existing) if existing.eq_ignore_ascii_case(ext) => path,
        _ => path.with_extension(ext),
    }
}
