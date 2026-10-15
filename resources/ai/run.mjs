#!/usr/bin/env node
/**
 * AI jobs: rewrite | titles | tags | generate | image-generate | image-edit |
 *          optimize-prompt | competitor-chat | viral-analyze
 * stdin JSON payload; last stdout line is JSON { ok, ... }
 */
import { mkdirSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

async function readStdin() {
  const chunks = []
  for await (const c of process.stdin) chunks.push(c)
  const raw = Buffer.concat(chunks).toString('utf8').trim()
  if (!raw) return {}
  return JSON.parse(raw)
}

function llmCfg(llm = {}) {
  return {
    baseUrl: String(llm.baseUrl || llm.base_url || '').trim(),
    apiKey: String(llm.apiKey || llm.api_key || '').trim(),
    model: String(llm.model || '').trim(),
    timeoutSec: Number(llm.timeoutSec || llm.timeout_sec || 60),
  }
}

function chatUrl(base) {
  const b = normalizeOpenAiBase(base)
  // DeepSeek 官方示例：https://api.deepseek.com/chat/completions（无强制 /v1）
  if (/api\.deepseek\.com$/i.test(b)) {
    return `${b}/chat/completions`
  }
  if (b.endsWith('/v1') || b.includes('/compatible-mode/v1') || b.endsWith('/paas/v4')) {
    return `${b}/chat/completions`
  }
  return `${b}/v1/chat/completions`
}

function normalizeOpenAiBase(base) {
  let b = String(base || '')
    .trim()
    .replace(/\/+$/, '')
  if (!b) return ''
  // 误填到具体 path 时剥掉，避免 /v1/chat/completions/v1/images/...
  b = b.replace(/\/chat\/completions$/i, '')
  b = b.replace(/\/images\/(generations|edits)$/i, '')
  b = b.replace(/\/images$/i, '')
  b = b.replace(/\/+$/, '')
  return b
}

function imagesApiRoot(base) {
  const b = normalizeOpenAiBase(base)
  if (!b) return ''
  if (b.endsWith('/v1') || b.includes('/compatible-mode/v1') || b.endsWith('/paas/v4')) {
    return `${b}/images`
  }
  // FlyMux / 多数中转：根域名需补 /v1
  // 文档：https://api.flymux.com/v1
  return `${b}/v1/images`
}

function imagesGenerationsUrl(base) {
  return `${imagesApiRoot(base)}/generations`
}

function imagesEditsUrl(base) {
  return `${imagesApiRoot(base)}/edits`
}

function mimeToExt(mime) {
  const m = String(mime || '').toLowerCase()
  if (m.includes('jpeg') || m.includes('jpg')) return 'jpg'
  if (m.includes('webp')) return 'webp'
  return 'png'
}

/** 把 b64 落盘，避免数 MB data URL 经 Tauri IPC 回传卡住 WebView（macOS 彩蛋） */
function persistB64Image(b64, mime = 'image/png') {
  const dir = join(tmpdir(), 'ai-content-factory-images')
  mkdirSync(dir, { recursive: true })
  const ext = mimeToExt(mime)
  const file = join(
    dir,
    `gen-${Date.now()}-${Math.random().toString(36).slice(2, 8)}.${ext}`,
  )
  writeFileSync(file, Buffer.from(String(b64 || ''), 'base64'))
  return file
}

function parseImageResult(data, model) {
  const item = Array.isArray(data?.data) ? data.data[0] : null
  if (!item) throw new Error('图片接口未返回 data')
  if (item.url) return { ok: true, url: String(item.url), model }
  if (item.b64_json) {
    // 勿把整图 base64 塞进 stdout JSON → IPC，会堵死前端主线程
    const path = persistB64Image(item.b64_json, 'image/png')
    return { ok: true, url: path, path, model }
  }
  throw new Error('图片接口未返回 url 或 b64_json')
}

/** 中转网关常见：模型未挂 generations / edits 渠道 */
function isImageChannelUnavailable(status, bodyText) {
  const t = String(bodyText || '')
  if (/No available channel/i.test(t) || /get_channel_failed/i.test(t)) return true
  if (Number(status) === 500 && /channel/i.test(t) && /model/i.test(t)) return true
  return false
}

function friendlyImageHttpError(kind, status, bodyText) {
  const raw = String(bodyText || '')
  let msg = ''
  try {
    const j = JSON.parse(raw)
    msg = String(j?.error?.message || j?.message || '').trim()
  } catch {
    msg = raw.replace(/\s+/g, ' ').trim()
  }
  if (isImageChannelUnavailable(status, raw) || isImageChannelUnavailable(status, msg)) {
    return kind === 'edit'
      ? '当前中转未开通「图生图」通道（模型无可用 edits 渠道）。请改用文生图，或在中转后台为该模型配置图片编辑通道。'
      : '当前中转未开通「文生图」通道（模型无可用 generations 渠道）。请在中转后台检查 gpt-image 分组/渠道。'
  }
  const brief = (msg || raw).slice(0, 220)
  return kind === 'edit'
    ? `Image Edit HTTP ${status}: ${brief}`
    : `Image API HTTP ${status}: ${brief}`
}

/** Tauri convertFileSrc → 本地路径（Node 无法访问 asset.localhost） */
function assetUrlToPath(src) {
  const s = String(src || '').trim()
  if (!s) return null
  try {
    if (s.startsWith('asset:')) {
      const u = new URL(s)
      let p = decodeURIComponent(u.pathname || '')
      if (process.platform === 'win32' && /^\/[A-Za-z]:/.test(p)) p = p.slice(1)
      return p || null
    }
    const u = new URL(s)
    if (u.hostname === 'asset.localhost' || u.hostname === 'localhost.asset') {
      let p = decodeURIComponent(u.pathname || '')
      if (process.platform === 'win32' && /^\/[A-Za-z]:/.test(p)) p = p.slice(1)
      return p || null
    }
  } catch {
    /* ignore */
  }
  return null
}

function fileUrlToPath(src) {
  const s = String(src || '').trim()
  if (!s.startsWith('file://')) return null
  let p = decodeURIComponent(s.slice('file://'.length))
  // file:///Users/... → /Users/...
  if (process.platform === 'win32' && /^\/[A-Za-z]:/.test(p)) p = p.slice(1)
  return p
}

function mimeFromPath(path) {
  const lower = String(path || '').toLowerCase()
  if (lower.endsWith('.jpg') || lower.endsWith('.jpeg')) return 'image/jpeg'
  if (lower.endsWith('.webp')) return 'image/webp'
  if (lower.endsWith('.gif')) return 'image/gif'
  return 'image/png'
}

async function fetchOrThrow(url, init, label) {
  try {
    return await fetch(url, init)
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    const cause = e && typeof e === 'object' && 'cause' in e ? e.cause : null
    const detail =
      cause instanceof Error
        ? cause.message
        : cause && typeof cause === 'object' && 'code' in cause
          ? String(cause.code)
          : cause
            ? String(cause)
            : ''
    throw new Error(
      `${label}失败: ${msg}${detail ? ` (${detail})` : ''} · ${String(url || '').slice(0, 160)}`,
    )
  }
}

async function loadBaseImage(payload) {
  const { readFileSync, existsSync } = await import('node:fs')
  const b64 = String(payload.imageBase64 || '').trim()
  if (b64) {
    const raw = b64.includes(',') ? b64.split(',', 1)[1] : b64
    const mime = String(payload.imageMime || 'image/png').trim() || 'image/png'
    return { bytes: Buffer.from(raw, 'base64'), mime }
  }
  let src = String(payload.imageUrl || payload.imageSrc || '').trim()
  if (!src) throw new Error('图生图需要底图')
  if (src.startsWith('data:')) {
    const m = /^data:([^;]+);base64,(.+)$/i.exec(src)
    if (!m) throw new Error('底图 data URL 无效')
    return { bytes: Buffer.from(m[2], 'base64'), mime: m[1] || 'image/png' }
  }

  // convertFileSrc / file:// → 本地读盘，避免 Node fetch asset.localhost 出现 fetch failed
  const local =
    assetUrlToPath(src) || fileUrlToPath(src) || (!/^https?:\/\//i.test(src) ? src : null)
  if (local) {
    if (!existsSync(local)) throw new Error(`底图文件不存在: ${local}`)
    return { bytes: readFileSync(local), mime: mimeFromPath(local) }
  }

  if (/^https?:\/\//i.test(src)) {
    const res = await fetchOrThrow(src, {}, '下载底图')
    if (!res.ok) throw new Error(`下载底图失败 HTTP ${res.status}`)
    const mime = (res.headers.get('content-type') || 'image/png').split(';')[0].trim()
    const bytes = Buffer.from(await res.arrayBuffer())
    return { bytes, mime: mime || 'image/png' }
  }

  throw new Error(`无法识别底图地址: ${src.slice(0, 120)}`)
}

async function chatComplete(llm, { system, user, messages }) {
  const cfg = llmCfg(llm)
  if (!cfg.baseUrl || !cfg.apiKey || !cfg.model) {
    throw new Error('请先配置文本大模型')
  }
  const msgs =
    messages ||
    [
      ...(system ? [{ role: 'system', content: system }] : []),
      { role: 'user', content: user },
    ]
  const ctrl = new AbortController()
  const t = setTimeout(() => ctrl.abort(), Math.max(5, cfg.timeoutSec) * 1000)
  try {
    const res = await fetchOrThrow(
      chatUrl(cfg.baseUrl),
      {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${cfg.apiKey}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          model: cfg.model,
          messages: msgs,
          temperature: 0.7,
        }),
        signal: ctrl.signal,
      },
      '请求文本模型',
    )
    const text = await res.text()
    if (!res.ok) {
      throw new Error(`LLM HTTP ${res.status}: ${text.slice(0, 300)}`)
    }
    const data = JSON.parse(text)
    const content = data?.choices?.[0]?.message?.content
    if (typeof content !== 'string') throw new Error('LLM 返回无内容')
    return content.trim()
  } finally {
    clearTimeout(t)
  }
}

async function chatCompleteWithImage(llm, { system, user, imageBytes, mime }) {
  const b64 = Buffer.from(imageBytes).toString('base64')
  const dataUrl = `data:${mime || 'image/png'};base64,${b64}`
  return chatComplete(llm, {
    messages: [
      { role: 'system', content: system },
      {
        role: 'user',
        content: [
          { type: 'text', text: user },
          { type: 'image_url', image_url: { url: dataUrl } },
        ],
      },
    ],
  })
}

function optimizeSystems(kind) {
  if (kind === 'copy') {
    return {
      system:
        '你是小红书文案改写指令优化专家。把用户的简短要求改写成清晰、可执行的改写指令（中文）。' +
        '保留核心意图，可补充语气、结构、emoji、转化等具体要求。不要解释，不要加引号或前缀，只输出最终指令。',
      userPrefix: '原始改写要求：',
    }
  }
  if (kind === 'image-edit') {
    return {
      system:
        '你是图像编辑指令优化专家。输出一条可直接用于图生图/图像编辑的中文指令。' +
        '保留用户核心诉求，可补充材质、光影、构图细节。不要解释，不要加引号或前缀，只输出最终指令。',
      userPrefix: '用户编辑意图：',
      visionSystem:
        '你是图像编辑指令优化专家。先理解附图内容，再结合用户意图，' +
        '输出一条可直接用于图生图/图像编辑的中文指令。' +
        '保留用户核心诉求，可补充与画面相关的材质、光影、构图细节。' +
        '不要解释，不要加引号或前缀，只输出最终指令。',
    }
  }
  // image-generate (default)
  return {
    system:
      '你是文生图提示词优化专家。保留用户意图，输出一条更具体、可直接用于图片生成的中文或中英混合提示词。' +
      '不要解释，不要加引号或前缀。',
    userPrefix: '原始提示词：',
  }
}

async function optimizePrompt(payload) {
  const kind = String(payload.kind || 'image-generate').trim()
  const text = String(payload.prompt || '').trim()
  const styleHint = String(payload.styleHint || '').trim()
  const systems = optimizeSystems(kind)
  if (!text && kind !== 'image-edit') {
    throw new Error('请先填写需要优化的内容')
  }

  // 图生图：优先带底图做视觉理解，失败则降级文本
  if (kind === 'image-edit' && (payload.imageUrl || payload.imageBase64)) {
    try {
      const { bytes, mime } = await loadBaseImage(payload)
      const intent = text || '根据附图生成可用于图生图的详细编辑指令'
      const user =
        `风格倾向：${styleHint || 'image edit instruction'}\n\n` +
        `用户编辑意图：\n${intent}\n\n请结合附图理解后改写编辑指令。`
      const out = await chatCompleteWithImage(payload.llm, {
        system: systems.visionSystem || systems.system,
        user,
        imageBytes: bytes,
        mime,
      })
      return { ok: true, prompt: out, usedVision: true }
    } catch {
      /* fall through */
    }
  }

  if (!text) throw new Error('请先填写需要优化的内容')
  let user = `${systems.userPrefix}\n${text}`
  if (styleHint) user = `风格倾向：${styleHint}\n\n${user}`
  if (kind === 'copy' && (payload.title || payload.body)) {
    user += `\n\n当前标题：${payload.title || '（无）'}\n正文摘要：\n${String(payload.body || '').slice(0, 400)}`
  }
  const out = await chatComplete(payload.llm, { system: systems.system, user })
  return { ok: true, prompt: out, usedVision: false }
}

async function rewrite(payload) {
  const system =
    '你是小红书内容运营编辑。保留事实信息，改写成自然、有种草感的中文笔记正文，不要输出标题或标签。'
  const instr = (payload.instruction || '').trim() || '更口语、更有转化感'
  const user = `改写要求：${instr}\n\n原标题：${payload.title || '（无）'}\n\n原文：\n${payload.body || '（空）'}\n\n请直接输出改写后的正文，不要加解释，不要输出标题，不要输出标签或 #话题。`
  const body = await chatComplete(payload.llm, { system, user })
  return { ok: true, body }
}

async function titles(payload) {
  const n = Math.max(1, Math.min(8, Number(payload.count || 3)))
  const system = '你是小红书标题优化专家。只输出标题列表，每行一个，不要编号说明。'
  const user = `请基于以下内容给出 ${n} 个更有吸引力的小红书标题（每条不超过 20 字）：\n当前标题：${payload.title || '（无）'}\n正文：\n${payload.body || '（空）'}\n\n请直接输出标题，每行一个。`
  const text = await chatComplete(payload.llm, { system, user })
  const list = text
    .split('\n')
    .map((ln) => ln.trim().replace(/^[\d.、)）\-\s]+/, '').trim())
    .filter(Boolean)
    .slice(0, n)
  return { ok: true, titles: list }
}

async function tags(payload) {
  const n = Math.max(1, Math.min(12, Number(payload.count || 5)))
  const system =
    '你是小红书话题标签专家。只输出 JSON 字符串数组，例如 ["标签1","标签2"]。禁止输出解释；标签不要带 # 号；每个标签 2～12 个字。'
  const existing = Array.isArray(payload.existing) ? payload.existing.join(', ') : '无'
  const user = `请给出 ${n} 个适合的小红书话题标签。\n标题：${payload.title || '（无）'}\n正文：\n${payload.body || '（空）'}\n已有标签：${existing}\n\n只回复 JSON 数组：`
  const text = await chatComplete(payload.llm, { system, user })
  let list = []
  try {
    const start = text.indexOf('[')
    const end = text.lastIndexOf(']')
    if (start >= 0 && end > start) {
      const parsed = JSON.parse(text.slice(start, end + 1))
      if (Array.isArray(parsed)) list = parsed.map((x) => String(x).trim().replace(/^#/, '')).filter(Boolean)
    }
  } catch {
    /* fallthrough */
  }
  if (!list.length) {
    list = text
      .split(/[,，\n#]+/)
      .map((t) => t.trim())
      .filter((t) => t.length >= 2 && t.length <= 20)
  }
  return { ok: true, tags: list.slice(0, n) }
}

/** 去掉正文末尾成行的 #话题（标签走独立 tags 字段，避免重复） */
function stripTrailingHashtagBlock(body) {
  let s = String(body || '').replace(/\r\n/g, '\n').trimEnd()
  // 连续删掉末尾「整行几乎全是 #tag」的行
  for (;;) {
    const m = s.match(/\n[ \t]*((?:#[^\s#]+[ \t]*){2,})\s*$/)
    if (!m) break
    s = s.slice(0, m.index).trimEnd()
  }
  // 末行只有零散 #tag
  s = s.replace(/\n[ \t]*(?:#[^\s#]+[ \t]*)+\s*$/u, '').trimEnd()
  return s
}

function extractHashtagsFromBody(body) {
  const found = String(body || '').match(/#[^\s#]+/g) || []
  return found.map((t) => t.replace(/^#/, '').trim()).filter(Boolean)
}

async function generate(payload) {
  const system =
    '你是小红书内容策划。请按 JSON 输出：{"title":"...","body":"...","tags":["..."]}，不要其它文字。' +
    '规则：body 只写种草正文，不要在正文末尾或正文中写 #话题标签；话题标签只放在 tags 数组（不要带 # 号，3～8 个）。'
  const user = `主题：${payload.topic || ''}\n参考：${payload.reference || '无'}\n要求：${payload.instruction || '自然种草'}\n\n只输出 JSON：`
  const text = await chatComplete(payload.llm, { system, user })
  const start = text.indexOf('{')
  const end = text.lastIndexOf('}')
  if (start < 0 || end <= start) {
    const bodyRaw = String(text || '')
    const fromBody = extractHashtagsFromBody(bodyRaw)
    return {
      ok: true,
      title: String(payload.topic || '').slice(0, 20),
      body: stripTrailingHashtagBlock(bodyRaw),
      tags: fromBody,
    }
  }
  try {
    const data = JSON.parse(text.slice(start, end + 1))
    // 客服多消息策略等自定义 JSON：原样放回 body，避免误当成笔记稿清空
    if (data && (data.strategy != null || data.replies != null || data.reply != null) && data.body == null) {
      return {
        ok: true,
        title: String(payload.topic || '').slice(0, 40),
        body: text.slice(start, end + 1),
        tags: [],
      }
    }
    const bodyRaw = String(data.body || '')
    const tagList = Array.isArray(data.tags)
      ? data.tags.map((t) => String(t).trim().replace(/^#/, '')).filter(Boolean)
      : []
    const merged = [...tagList]
    for (const t of extractHashtagsFromBody(bodyRaw)) {
      if (!merged.includes(t)) merged.push(t)
    }
    return {
      ok: true,
      title: String(data.title || payload.topic || '').slice(0, 40),
      body: stripTrailingHashtagBlock(bodyRaw),
      tags: merged.slice(0, 12),
    }
  } catch {
    const bodyRaw = String(text || '')
    return {
      ok: true,
      title: String(payload.topic || '').slice(0, 20),
      body: stripTrailingHashtagBlock(bodyRaw),
      tags: extractHashtagsFromBody(bodyRaw),
    }
  }
}

/** 自由文本补全（客服回复 / 策略 JSON 等，不强制笔记结构） */
async function complete(payload) {
  const system =
    String(payload.system || '').trim() ||
    '你是小红书店铺客服助手。按用户要求直接输出可用结果，不要解释过程。'
  const user = String(payload.user || payload.instruction || '').trim()
  if (!user) throw new Error('complete 缺少 user/instruction')
  const text = await chatComplete(payload.llm, { system, user })
  return { ok: true, text: String(text || '').trim(), body: String(text || '').trim() }
}

async function imageGenerate(payload) {
  const cfg = llmCfg(payload.llm)
  if (!cfg.baseUrl || !cfg.apiKey || !cfg.model) {
    throw new Error('请先配置图片大模型（如 gpt-image-2）')
  }
  const prompt = String(payload.prompt || '').trim()
  if (!prompt) throw new Error('请填写配图提示词')
  const size = String(payload.size || '1024x1024').trim() || '1024x1024'
  const quality = String(payload.quality || 'medium').trim() || 'medium'
  const url = imagesGenerationsUrl(cfg.baseUrl)
  const ctrl = new AbortController()
  const t = setTimeout(() => ctrl.abort(), Math.max(30, cfg.timeoutSec || 180) * 1000)
  try {
    // 与 FlyMux / OpenAI 兼容文档一致：response_format=b64_json，避免只返回远端 url
    const body = {
      model: cfg.model,
      prompt,
      n: 1,
      size,
      response_format: 'b64_json',
    }
    // gpt-image-* supports quality; dall-e-3 uses different params
    if (/^gpt-image/i.test(cfg.model)) {
      body.quality = quality
    }
    const res = await fetchOrThrow(
      url,
      {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${cfg.apiKey}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify(body),
        signal: ctrl.signal,
      },
      '文生图请求',
    )
    const text = await res.text()
    if (!res.ok) {
      throw new Error(friendlyImageHttpError('generate', res.status, text))
    }
    return parseImageResult(JSON.parse(text), cfg.model)
  } finally {
    clearTimeout(t)
  }
}

async function imageEdit(payload) {
  const cfg = llmCfg(payload.llm)
  if (!cfg.baseUrl || !cfg.apiKey || !cfg.model) {
    throw new Error('请先配置图片大模型（如 gpt-image-2）')
  }
  const prompt = String(payload.prompt || '').trim()
  if (!prompt) throw new Error('请填写编辑指令')
  const { bytes, mime } = await loadBaseImage(payload)
  if (!bytes?.length) throw new Error('底图为空')
  const size = String(payload.size || '').trim()
  const quality = String(payload.quality || '').trim()
  const url = imagesEditsUrl(cfg.baseUrl)
  const ctrl = new AbortController()
  const t = setTimeout(() => ctrl.abort(), Math.max(30, cfg.timeoutSec || 180) * 1000)
  try {
    const { File } = await import('node:buffer')
    const form = new FormData()
    form.append('model', cfg.model)
    form.append('prompt', prompt)
    form.append('n', '1')
    form.append('response_format', 'b64_json')
    if (size) form.append('size', size)
    if (quality && /^gpt-image/i.test(cfg.model)) form.append('quality', quality)
    // 使用 File + 原始 Buffer；Uint8Array.from 大图既慢又易触发异常
    const file = new File([bytes], `base.${mimeToExt(mime)}`, {
      type: mime || 'image/png',
    })
    form.append('image', file)
    const res = await fetchOrThrow(
      url,
      {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${cfg.apiKey}`,
          Accept: 'application/json',
        },
        body: form,
        signal: ctrl.signal,
      },
      '图生图请求',
    )
    const text = await res.text()
    if (!res.ok) {
      // FlyMux 等：gpt-image-2 往往只开了 generations，edits 无渠道 → 降级文生图
      if (isImageChannelUnavailable(res.status, text)) {
        const genPrompt =
          `根据以下编辑指令生成一张完整产品展示图（图生图通道不可用，已按文生图执行）：\n${prompt}`
        const out = await imageGenerate({
          ...payload,
          prompt: genPrompt,
          size: size || '1024x1024',
          quality: quality || 'medium',
        })
        return { ...out, fallback: 'generate' }
      }
      throw new Error(friendlyImageHttpError('edit', res.status, text))
    }
    return parseImageResult(JSON.parse(text), cfg.model)
  } finally {
    clearTimeout(t)
  }
}

async function competitorChat(payload) {
  const message = String(payload.message || '').trim()
  if (!message) throw new Error('请输入问题')
  if (message.length > 4000) throw new Error('问题过长')
  const context = typeof payload.context === 'string' ? payload.context : JSON.stringify(payload.context || {})
  const history = Array.isArray(payload.history) ? payload.history.slice(-16) : []
  const system =
    '你是小红书竞品监控智能助手。只能基于用户提供的「监控数据 JSON」回答。给出可执行的运营建议，语气专业简洁。若数据不足请说明。'
  const messages = [
    { role: 'system', content: system },
    { role: 'user', content: `监控数据 JSON：\n${context}` },
    ...history.map((m) => ({
      role: m.role === 'assistant' ? 'assistant' : 'user',
      content: String(m.content || ''),
    })),
    { role: 'user', content: message },
  ]
  const reply = await chatComplete(payload.llm, { messages })
  return { ok: true, reply }
}

/** 内容库爆款分析：单条笔记或一组笔记 */
async function viralAnalyze(payload) {
  const scope = String(payload.scope || 'item').trim() === 'group' ? 'group' : 'item'
  const platform = String(payload.platform || 'xhs').trim() || 'xhs'
  const platformLabel = platform === 'douyin' ? '抖音' : '小红书'
  const items = Array.isArray(payload.items) ? payload.items : []
  if (!items.length) throw new Error('缺少待分析内容')

  const compact = items.slice(0, scope === 'group' ? 12 : 1).map((it, i) => {
    const desc = String(it.desc || it.body || '').trim().slice(0, 400)
    return {
      i: i + 1,
      title: String(it.title || '').slice(0, 80),
      author: String(it.authorName || it.author || '').slice(0, 40),
      likes: Number(it.likes) || 0,
      collects: Number(it.collects) || 0,
      comments: Number(it.comments) || 0,
      shares: Number(it.shares) || 0,
      desc,
    }
  })

  const system =
    scope === 'group'
      ? `你是${platformLabel}内容策略专家。根据一组已入库笔记的标题、互动数据与正文摘要，分析整组为何容易出爆款，并给出可执行创作建议。只输出 JSON，不要 Markdown。`
      : `你是${platformLabel}内容策略专家。根据单条笔记的标题、互动数据与正文摘要，分析其爆火原因，并给出可执行创作建议。只输出 JSON，不要 Markdown。`

  const schemaHint =
    '{"summary":"2～4句总评","hooks":["爆点1","爆点2"],"structure":"内容结构观察","suggestions":["建议1","建议2","建议3"],"score":0到100的整数}'

  const user =
    scope === 'group'
      ? `请分析以下一组${platformLabel}笔记（共 ${compact.length} 条）的共性爆款因素：\n${JSON.stringify(compact)}\n\n只输出 JSON：${schemaHint}`
      : `请分析以下这条${platformLabel}笔记为何容易火：\n${JSON.stringify(compact[0])}\n\n只输出 JSON：${schemaHint}`

  const text = await chatComplete(payload.llm, { system, user })
  const start = text.indexOf('{')
  const end = text.lastIndexOf('}')
  if (start < 0 || end <= start) throw new Error('模型未返回有效 JSON')

  let data
  try {
    data = JSON.parse(text.slice(start, end + 1))
  } catch {
    throw new Error('解析分析结果失败')
  }

  const hooks = Array.isArray(data.hooks) ? data.hooks.map((x) => String(x).trim()).filter(Boolean) : []
  const suggestions = Array.isArray(data.suggestions)
    ? data.suggestions.map((x) => String(x).trim()).filter(Boolean)
    : []
  let score = Number(data.score)
  if (!Number.isFinite(score)) score = 0
  score = Math.max(0, Math.min(100, Math.round(score)))

  return {
    ok: true,
    scope,
    summary: String(data.summary || '').trim(),
    hooks: hooks.slice(0, 8),
    structure: String(data.structure || '').trim(),
    suggestions: suggestions.slice(0, 8),
    score,
  }
}

const action = process.argv[2] || ''
const payload = await readStdin()

try {
  let out
  switch (action) {
    case 'rewrite':
      out = await rewrite(payload)
      break
    case 'titles':
      out = await titles(payload)
      break
    case 'tags':
      out = await tags(payload)
      break
    case 'generate':
      out = await generate(payload)
      break
    case 'complete':
      out = await complete(payload)
      break
    case 'image-generate':
      out = await imageGenerate(payload)
      break
    case 'image-edit':
      out = await imageEdit(payload)
      break
    case 'optimize-prompt':
      out = await optimizePrompt(payload)
      break
    case 'competitor-chat':
      out = await competitorChat(payload)
      break
    case 'viral-analyze':
      out = await viralAnalyze(payload)
      break
    default:
      out = { ok: false, error: `unknown action: ${action}` }
  }
  console.log(JSON.stringify(out))
  process.exit(out.ok ? 0 : 1)
} catch (e) {
  console.log(JSON.stringify({ ok: false, error: e instanceof Error ? e.message : String(e) }))
  process.exit(1)
}
