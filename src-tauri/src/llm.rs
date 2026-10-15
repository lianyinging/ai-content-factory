//! LLM provider CRUD + connectivity test (OpenAI-compatible).

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

use crate::library::LibraryState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmProvider {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub modality: String,
    pub base_url: String,
    pub api_key_hint: String,
    pub has_api_key: bool,
    pub default_model: String,
    pub extra_models: String,
    pub timeout_sec: i64,
    pub enabled: bool,
    pub is_default: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmRuntime {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub modality: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout_sec: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertLlmProvider {
    pub id: Option<String>,
    pub name: String,
    pub provider: String,
    pub modality: Option<String>,
    pub base_url: String,
    pub api_key: Option<String>,
    pub default_model: String,
    pub extra_models: Option<String>,
    pub timeout_sec: Option<i64>,
    pub enabled: Option<bool>,
    pub is_default: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmTestResult {
    pub ok: bool,
    pub latency_ms: i64,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmBalanceItem {
    pub currency: String,
    pub total_balance: String,
    pub granted_balance: String,
    pub topped_up_balance: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmBalanceResult {
    pub ok: bool,
    pub supported: bool,
    pub available: Option<bool>,
    pub message: String,
    pub items: Vec<LlmBalanceItem>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn key_hint(api_key: &str) -> String {
    let t = api_key.trim();
    if t.is_empty() {
        return String::new();
    }
    if t.len() <= 4 {
        return t.to_string();
    }
    format!("***{}", &t[t.len() - 4..])
}

fn normalize_modality(raw: &str) -> Result<String, String> {
    match raw.trim() {
        "" | "text" => Ok("text".into()),
        "image" => Ok("image".into()),
        "video" => Ok("video".into()),
        "multimodal" => Ok("multimodal".into()),
        other => Err(format!("未知 modality: {other}")),
    }
}

fn can_be_default(modality: &str) -> bool {
    modality == "text" || modality == "multimodal"
}

fn map_row(
    id: String,
    name: String,
    provider: String,
    modality: String,
    base_url: String,
    api_key: String,
    api_key_hint: String,
    default_model: String,
    extra_models: String,
    timeout_sec: i64,
    enabled: i64,
    is_default: i64,
    updated_at: i64,
) -> LlmProvider {
    LlmProvider {
        id,
        name,
        provider,
        modality,
        base_url,
        api_key_hint,
        has_api_key: !api_key.trim().is_empty(),
        default_model,
        extra_models,
        timeout_sec,
        enabled: enabled != 0,
        is_default: is_default != 0,
        updated_at,
    }
}

fn chat_url(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    // DeepSeek 官方：https://api.deepseek.com/chat/completions
    if b.eq_ignore_ascii_case("https://api.deepseek.com")
        || b.eq_ignore_ascii_case("http://api.deepseek.com")
    {
        return format!("{b}/chat/completions");
    }
    if b.ends_with("/v1") {
        format!("{b}/chat/completions")
    } else if b.contains("/compatible-mode/v1") || b.ends_with("/paas/v4") {
        format!("{b}/chat/completions")
    } else {
        format!("{b}/v1/chat/completions")
    }
}

fn models_url(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.eq_ignore_ascii_case("https://api.deepseek.com")
        || b.eq_ignore_ascii_case("http://api.deepseek.com")
    {
        return format!("{b}/models");
    }
    if b.ends_with("/v1") {
        format!("{b}/models")
    } else if b.contains("/compatible-mode/v1") || b.ends_with("/paas/v4") {
        format!("{b}/models")
    } else {
        format!("{b}/v1/models")
    }
}

fn api_origin(base_url: &str) -> String {
    let mut base = base_url.trim().trim_end_matches('/').to_string();
    for suffix in ["/chat/completions", "/compatible-mode/v1", "/v1"] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped.trim_end_matches('/').to_string();
            break;
        }
    }
    base
}

fn balance_url(provider: &str, base_url: &str) -> Result<String, String> {
    match provider.trim() {
        // DeepSeek 官方：GET https://api.deepseek.com/user/balance
        "deepseek" => Ok(format!("{}/user/balance", api_origin(base_url))),
        other => Err(format!("平台「{other}」暂不支持余额查询")),
    }
}

#[tauri::command]
pub fn list_llm_providers(db: State<'_, LibraryState>) -> Result<Vec<LlmProvider>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, provider, modality, base_url, api_key, api_key_hint, default_model,
                    extra_models, timeout_sec, enabled, is_default, updated_at
             FROM llm_provider
             ORDER BY is_default DESC, updated_at DESC, name ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(map_row(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
                row.get(10)?,
                row.get(11)?,
                row.get(12)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    for r in rows {
        items.push(r.map_err(|e| e.to_string())?);
    }
    Ok(items)
}

#[tauri::command]
pub fn upsert_llm_provider(
    db: State<'_, LibraryState>,
    body: UpsertLlmProvider,
) -> Result<LlmProvider, String> {
    let name = body.name.trim().to_string();
    let provider = body.provider.trim().to_string();
    let modality = normalize_modality(body.modality.as_deref().unwrap_or("text"))?;
    let base_url = body.base_url.trim().to_string();
    let default_model = body.default_model.trim().to_string();
    if name.is_empty() || provider.is_empty() || base_url.is_empty() || default_model.is_empty() {
        return Err("名称、平台、Base URL、默认模型均为必填".into());
    }
    let extra_models = body
        .extra_models
        .as_deref()
        .unwrap_or("[]")
        .trim()
        .to_string();
    let timeout_sec = body.timeout_sec.unwrap_or(120).max(5);
    let enabled = if body.enabled.unwrap_or(true) { 1 } else { 0 };
    let make_default = body.is_default.unwrap_or(false) && can_be_default(&modality);
    let ts = now_ms();
    let id = body
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("p_{ts}"));

    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let existing: Option<(String, String)> = conn
        .query_row(
            "SELECT api_key, api_key_hint FROM llm_provider WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    let incoming_key = body.api_key.as_deref().unwrap_or("").trim().to_string();
    let (api_key, hint) = if !incoming_key.is_empty() {
        (incoming_key.clone(), key_hint(&incoming_key))
    } else if let Some((old_key, old_hint)) = existing.clone() {
        (old_key, old_hint)
    } else {
        return Err("新建供应商需要填写 API Key".into());
    };

    if make_default {
        conn.execute(
            "UPDATE llm_provider SET is_default = 0 WHERE modality IN ('text', 'multimodal')",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    let is_default = if make_default {
        1
    } else if existing.is_none() && can_be_default(&modality) {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM llm_provider WHERE modality IN ('text', 'multimodal')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if count == 0 {
            1
        } else {
            0
        }
    } else if !can_be_default(&modality) {
        0
    } else {
        conn.query_row(
            "SELECT is_default FROM llm_provider WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or(0)
    };

    if existing.is_some() {
        conn.execute(
            "UPDATE llm_provider SET
                name=?2, provider=?3, modality=?4, base_url=?5, api_key=?6, api_key_hint=?7,
                default_model=?8, extra_models=?9, timeout_sec=?10, enabled=?11,
                is_default=?12, updated_at=?13
             WHERE id=?1",
            params![
                id,
                name,
                provider,
                modality,
                base_url,
                api_key,
                hint,
                default_model,
                extra_models,
                timeout_sec,
                enabled,
                is_default,
                ts
            ],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "INSERT INTO llm_provider (
                id, name, provider, modality, base_url, api_key, api_key_hint, default_model,
                extra_models, timeout_sec, enabled, is_default, created_at, updated_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13)",
            params![
                id,
                name,
                provider,
                modality,
                base_url,
                api_key,
                hint,
                default_model,
                extra_models,
                timeout_sec,
                enabled,
                is_default,
                ts
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    conn.query_row(
        "SELECT id, name, provider, modality, base_url, api_key, api_key_hint, default_model,
                extra_models, timeout_sec, enabled, is_default, updated_at
         FROM llm_provider WHERE id = ?1",
        params![id],
        |row| {
            Ok(map_row(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
                row.get(10)?,
                row.get(11)?,
                row.get(12)?,
            ))
        },
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_llm_provider(db: State<'_, LibraryState>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let was_default: i64 = conn
        .query_row(
            "SELECT is_default FROM llm_provider WHERE id = ?1",
            params![id.trim()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or(0);
    conn.execute("DELETE FROM llm_provider WHERE id = ?1", params![id.trim()])
        .map_err(|e| e.to_string())?;
    if was_default != 0 {
        if let Ok(next_id) = conn.query_row(
            "SELECT id FROM llm_provider WHERE modality IN ('text', 'multimodal') ORDER BY updated_at DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        ) {
            let _ = conn.execute(
                "UPDATE llm_provider SET is_default = 1 WHERE id = ?1",
                params![next_id],
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub fn set_default_llm_provider(db: State<'_, LibraryState>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let modality: String = conn
        .query_row(
            "SELECT modality FROM llm_provider WHERE id = ?1",
            params![id.trim()],
            |row| row.get(0),
        )
        .map_err(|_| "供应商不存在".to_string())?;
    if !can_be_default(&modality) {
        return Err("只有文本/多模态模型可设为默认".into());
    }
    conn.execute(
        "UPDATE llm_provider SET is_default = 0 WHERE modality IN ('text', 'multimodal')",
        [],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE llm_provider SET is_default = 1, updated_at = ?2 WHERE id = ?1",
        params![id.trim(), now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_llm_runtime_by_modality(
    db: State<'_, LibraryState>,
    modality: String,
) -> Result<Option<LlmRuntime>, String> {
    let want = normalize_modality(&modality)?;
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, provider, modality, base_url, api_key, default_model, timeout_sec
             FROM llm_provider WHERE enabled = 1 AND length(trim(api_key)) > 0
             ORDER BY is_default DESC, updated_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<LlmRuntime> = stmt
        .query_map([], |row| {
            Ok(LlmRuntime {
                id: row.get(0)?,
                name: row.get(1)?,
                provider: row.get(2)?,
                modality: row.get(3)?,
                base_url: row.get(4)?,
                api_key: row.get(5)?,
                model: row.get(6)?,
                timeout_sec: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    if let Some(r) = rows.iter().find(|r| r.modality == want) {
        return Ok(Some(r.clone()));
    }
    if want == "text" || want == "multimodal" {
        if let Some(r) = rows.iter().find(|r| can_be_default(&r.modality)) {
            return Ok(Some(r.clone()));
        }
    }
    Ok(None)
}

fn is_image_model_name(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    m.starts_with("gpt-image") || m.starts_with("dall-e")
}

#[tauri::command]
pub fn test_llm_provider(db: State<'_, LibraryState>, id: String) -> Result<LlmTestResult, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let (base_url, api_key, model, timeout_sec, modality): (String, String, String, i64, String) =
        conn
            .query_row(
                "SELECT base_url, api_key, default_model, timeout_sec, modality FROM llm_provider WHERE id = ?1",
                params![id.trim()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .map_err(|_| "供应商不存在".to_string())?;
    drop(conn);

    if api_key.trim().is_empty() {
        return Ok(LlmTestResult {
            ok: false,
            latency_ms: 0,
            message: "未配置 API Key".into(),
        });
    }

    let modality = normalize_modality(&modality).unwrap_or_else(|_| "text".into());
    let image_channel = modality == "image" || modality == "video" || is_image_model_name(&model);

    let timeout = timeout_sec.clamp(5, 60) as u64;
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(timeout))
        .build();
    let auth = format!("Bearer {}", api_key.trim());
    let started = SystemTime::now();

    match agent
        .get(&models_url(&base_url))
        .set("Authorization", &auth)
        .set("Accept", "application/json")
        .call()
    {
        Ok(resp) if (200..300).contains(&resp.status()) => {
            let latency_ms = started.elapsed().map(|d| d.as_millis() as i64).unwrap_or(0);
            return Ok(LlmTestResult {
                ok: true,
                latency_ms,
                message: format!("连通成功（models，{latency_ms} ms）"),
            });
        }
        Ok(resp) if resp.status() == 401 || resp.status() == 403 => {
            let status = resp.status();
            let text = resp.into_string().unwrap_or_default();
            let latency_ms = started.elapsed().map(|d| d.as_millis() as i64).unwrap_or(0);
            return Ok(LlmTestResult {
                ok: false,
                latency_ms,
                message: format!(
                    "HTTP {status}: {}",
                    text.chars().take(240).collect::<String>()
                ),
            });
        }
        Err(err) if image_channel => {
            let latency_ms = started.elapsed().map(|d| d.as_millis() as i64).unwrap_or(0);
            return Ok(LlmTestResult {
                ok: false,
                latency_ms,
                message: format!(
                    "探测失败（{timeout}s 内）：{err}。图片/视频供应商不一定提供 /models，请确认 Base URL 与 Key。"
                ),
            });
        }
        _ => {}
    }

    // gpt-image-* / dall-e / 图片模态走 Images API，不探测 chat/completions
    if image_channel {
        let latency_ms = started.elapsed().map(|d| d.as_millis() as i64).unwrap_or(0);
        return Ok(LlmTestResult {
            ok: false,
            latency_ms,
            message: format!(
                "未能通过 /models 验证（{latency_ms} ms）。图片通道（如 gpt-image-2）不走 chat/completions，请确认 Base URL 可访问。"
            ),
        });
    }

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 1,
    });
    let chat_started = SystemTime::now();
    match agent
        .post(&chat_url(&base_url))
        .set("Authorization", &auth)
        .set("Content-Type", "application/json")
        .send_json(&body)
    {
        Ok(resp) => {
            let status = resp.status();
            let latency_ms = chat_started
                .elapsed()
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            if (200..300).contains(&status) {
                Ok(LlmTestResult {
                    ok: true,
                    latency_ms,
                    message: format!("连通成功（chat，{latency_ms} ms）"),
                })
            } else {
                let text = resp.into_string().unwrap_or_default();
                Ok(LlmTestResult {
                    ok: false,
                    latency_ms,
                    message: format!(
                        "HTTP {status}: {}",
                        text.chars().take(240).collect::<String>()
                    ),
                })
            }
        }
        Err(err) => {
            let latency_ms = chat_started
                .elapsed()
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            Ok(LlmTestResult {
                ok: false,
                latency_ms,
                message: format!("探测失败：{err}"),
            })
        }
    }
}

#[tauri::command]
pub fn fetch_llm_balance(db: State<'_, LibraryState>, id: String) -> Result<LlmBalanceResult, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let (provider, base_url, api_key, timeout_sec): (String, String, String, i64) = conn
        .query_row(
            "SELECT provider, base_url, api_key, timeout_sec FROM llm_provider WHERE id = ?1",
            params![id.trim()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| "供应商不存在".to_string())?;
    drop(conn);

    if api_key.trim().is_empty() {
        return Ok(LlmBalanceResult {
            ok: false,
            supported: true,
            available: None,
            message: "未配置 API Key".into(),
            items: vec![],
        });
    }

    let url = match balance_url(&provider, &base_url) {
        Ok(u) => u,
        Err(msg) => {
            return Ok(LlmBalanceResult {
                ok: false,
                supported: false,
                available: None,
                message: msg,
                items: vec![],
            });
        }
    };

    let timeout = timeout_sec.clamp(5, 60) as u64;
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(timeout))
        .build();
    match agent
        .get(&url)
        .set("Authorization", &format!("Bearer {}", api_key.trim()))
        .set("Accept", "application/json")
        .call()
    {
        Ok(resp) => {
            let status = resp.status();
            let text = resp.into_string().unwrap_or_default();
            if !(200..300).contains(&status) {
                return Ok(LlmBalanceResult {
                    ok: false,
                    supported: true,
                    available: None,
                    message: format!(
                        "HTTP {status}: {}",
                        text.chars().take(240).collect::<String>()
                    ),
                    items: vec![],
                });
            }
            let parsed: serde_json::Value =
                serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
            let available = parsed.get("is_available").and_then(|v| v.as_bool());
            let mut items = Vec::new();
            if let Some(arr) = parsed.get("balance_infos").and_then(|v| v.as_array()) {
                for entry in arr {
                    items.push(LlmBalanceItem {
                        currency: entry
                            .get("currency")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        total_balance: entry
                            .get("total_balance")
                            .and_then(|v| v.as_str())
                            .unwrap_or("0")
                            .to_string(),
                        granted_balance: entry
                            .get("granted_balance")
                            .and_then(|v| v.as_str())
                            .unwrap_or("0")
                            .to_string(),
                        topped_up_balance: entry
                            .get("topped_up_balance")
                            .and_then(|v| v.as_str())
                            .unwrap_or("0")
                            .to_string(),
                    });
                }
            }
            let summary = if items.is_empty() {
                "已查询，但未返回余额明细".to_string()
            } else {
                items
                    .iter()
                    .map(|item| format!("{} {}", item.currency, item.total_balance))
                    .collect::<Vec<_>>()
                    .join(" · ")
            };
            let avail_hint = match available {
                Some(true) => "可用",
                Some(false) => "余额不足",
                None => "",
            };
            let message = if avail_hint.is_empty() {
                summary
            } else {
                format!("{summary}（{avail_hint}）")
            };
            Ok(LlmBalanceResult {
                ok: true,
                supported: true,
                available,
                message,
                items,
            })
        }
        Err(err) => Ok(LlmBalanceResult {
            ok: false,
            supported: true,
            available: None,
            message: err.to_string(),
            items: vec![],
        }),
    }
}
