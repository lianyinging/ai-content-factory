//! Seed + user product catalog for draft workshop.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductItem {
    pub id: String,
    pub sku: String,
    pub name: String,
    #[serde(default)]
    pub spec: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub unit: String,
    /// Relative path inside catalog root, e.g. `images/xxx.jpg`
    pub image: String,
    /// Absolute filesystem path for UI / draft attachment
    #[serde(default)]
    pub image_abs_path: String,
    /// `seed` | `user`
    pub source: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductUpsertBody {
    pub sku: Option<String>,
    pub name: String,
    pub spec: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub unit: Option<String>,
    /// data URL or raw base64
    pub image_base64: Option<String>,
    /// Optional absolute path if already on disk
    pub image_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    products: Vec<CatalogProductRaw>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct CatalogProductRaw {
    id: String,
    sku: String,
    name: String,
    #[serde(default)]
    spec: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    unit: String,
    image: String,
    #[serde(default = "seed_source")]
    source: String,
}

fn seed_source() -> String {
    "seed".into()
}

fn products_seed_dir(app: &AppHandle) -> PathBuf {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/products"),
        app.path()
            .resource_dir()
            .map(|p| p.join("resources/products"))
            .unwrap_or_default(),
        app.path()
            .resource_dir()
            .map(|p| p.join("products"))
            .unwrap_or_default(),
    ];
    for c in &candidates {
        if c.join("catalog.json").is_file() {
            return c.canonicalize().unwrap_or_else(|_| c.clone());
        }
    }
    candidates[0].clone()
}

fn products_user_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?
        .join("products");
    fs::create_dir_all(dir.join("images")).map_err(|e| format!("mkdir products: {e}"))?;
    Ok(dir)
}

fn read_catalog(path: &Path) -> Result<Vec<CatalogProductRaw>, String> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).map_err(|e| format!("读 catalog 失败: {e}"))?;
    let file: CatalogFile =
        serde_json::from_str(&text).map_err(|e| format!("解析 catalog 失败: {e}"))?;
    let _ = file.version;
    Ok(file.products)
}

fn write_user_catalog(path: &Path, products: &[CatalogProductRaw]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let body = serde_json::json!({
        "version": 1,
        "products": products,
    });
    fs::write(
        path,
        serde_json::to_string_pretty(&body).map_err(|e| e.to_string())? + "\n",
    )
    .map_err(|e| format!("写 user-catalog 失败: {e}"))
}

fn resolve_image(root: &Path, rel: &str) -> String {
    let p = root.join(rel);
    p.canonicalize()
        .unwrap_or(p)
        .to_string_lossy()
        .to_string()
}

fn to_item(raw: CatalogProductRaw, root: &Path) -> ProductItem {
    let abs = resolve_image(root, &raw.image);
    ProductItem {
        id: raw.id,
        sku: raw.sku,
        name: raw.name,
        spec: raw.spec,
        description: raw.description,
        category: raw.category,
        unit: raw.unit,
        image: raw.image,
        image_abs_path: abs,
        source: raw.source,
    }
}

fn decode_image_bytes(b64: &str) -> Result<(Vec<u8>, String), String> {
    let raw = if let Some((_, data)) = b64.split_once(',') {
        data
    } else {
        b64.trim()
    };
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, raw)
        .map_err(|e| format!("base64 解码失败: {e}"))?;
    let ext = if b64.contains("image/png") {
        "png"
    } else if b64.contains("image/webp") {
        "webp"
    } else if b64.contains("image/gif") {
        "gif"
    } else {
        "jpg"
    };
    Ok((bytes, ext.into()))
}

fn sanitize_sku(s: &str) -> String {
    let t: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if t.is_empty() {
        format!("p{}", chrono_like_id())
    } else {
        t
    }
}

fn chrono_like_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms}")
}

#[tauri::command]
pub fn product_list(app: AppHandle) -> Result<Vec<ProductItem>, String> {
    let seed_root = products_seed_dir(&app);
    let user_root = products_user_dir(&app)?;

    let seed = read_catalog(&seed_root.join("catalog.json"))?;
    let user = read_catalog(&user_root.join("user-catalog.json"))?;

    let mut map: HashMap<String, (CatalogProductRaw, PathBuf)> = HashMap::new();
    for p in seed {
        let key = p.sku.clone();
        map.insert(key, (p, seed_root.clone()));
    }
    for mut p in user {
        p.source = "user".into();
        let key = p.sku.clone();
        map.insert(key, (p, user_root.clone()));
    }

    let mut items: Vec<ProductItem> = map
        .into_values()
        .map(|(raw, root)| to_item(raw, &root))
        .collect();
    items.sort_by(|a, b| a.sku.cmp(&b.sku).then(a.name.cmp(&b.name)));
    Ok(items)
}

#[tauri::command]
pub fn product_upsert(app: AppHandle, body: ProductUpsertBody) -> Result<ProductItem, String> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err("品名不能为空".into());
    }
    let sku = sanitize_sku(
        body.sku
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(""),
    );

    let user_root = products_user_dir(&app)?;
    let images_dir = user_root.join("images");
    fs::create_dir_all(&images_dir).map_err(|e| format!("mkdir images: {e}"))?;

    let (bytes, ext) = if let Some(b64) = body
        .image_base64
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        decode_image_bytes(b64)?
    } else if let Some(path) = body
        .image_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let p = Path::new(path);
        if !p.is_file() {
            return Err(format!("图片不存在: {path}"));
        }
        let bytes = fs::read(p).map_err(|e| format!("读图失败: {e}"))?;
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg");
        let ext = if ext.eq_ignore_ascii_case("jpeg") {
            "jpg".into()
        } else {
            ext.to_ascii_lowercase()
        };
        (bytes, ext)
    } else {
        return Err("请上传产品图".into());
    };

    let rel = format!("images/{sku}.{ext}");
    let dest = user_root.join(&rel);
    fs::write(&dest, &bytes).map_err(|e| format!("写图失败: {e}"))?;

    let raw = CatalogProductRaw {
        id: sku.clone(),
        sku: sku.clone(),
        name,
        spec: body.spec.unwrap_or_default().trim().to_string(),
        description: body.description.unwrap_or_default().trim().to_string(),
        category: body.category.unwrap_or_default().trim().to_string(),
        unit: body.unit.unwrap_or_default().trim().to_string(),
        image: rel,
        source: "user".into(),
    };

    let catalog_path = user_root.join("user-catalog.json");
    let mut list = read_catalog(&catalog_path)?;
    if let Some(idx) = list.iter().position(|p| p.sku == sku) {
        list[idx] = raw.clone();
    } else {
        list.push(raw.clone());
    }
    write_user_catalog(&catalog_path, &list)?;

    Ok(to_item(raw, &user_root))
}

#[tauri::command]
pub fn product_delete(app: AppHandle, sku: String) -> Result<Value, String> {
    let sku = sku.trim().to_string();
    if sku.is_empty() {
        return Err("缺少 sku".into());
    }
    let user_root = products_user_dir(&app)?;
    let catalog_path = user_root.join("user-catalog.json");
    let mut list = read_catalog(&catalog_path)?;
    let Some(idx) = list.iter().position(|p| p.sku == sku) else {
        return Err("只能删除自定义上传的产品".into());
    };
    let removed = list.remove(idx);
    if removed.source != "user" && !catalog_path.exists() {
        return Err("只能删除自定义上传的产品".into());
    }
    let img = user_root.join(&removed.image);
    let _ = fs::remove_file(&img);
    write_user_catalog(&catalog_path, &list)?;
    Ok(serde_json::json!({ "ok": true, "sku": sku }))
}
