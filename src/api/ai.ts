import { invoke } from '@tauri-apps/api/core'
import {
  llmPayload,
  requireImageLlm,
  requireTextLlm,
  type LlmRuntime,
} from '@/llm/llmProviders'

type NodeJobResult = {
  ok: boolean
  exitCode: number
  result?: Record<string, unknown> | null
  stdout: string
  stderr: string
  error?: string | null
}

async function aiRun(action: string, payload: Record<string, unknown>, timeoutSecs = 120) {
  const res = await invoke<NodeJobResult>('ai_run', {
    req: { action, payload, timeoutSecs },
  })
  if (!res.ok) {
    const err = res.error || (res.result && String(res.result.error || '')) || 'AI 请求失败'
    throw new Error(err)
  }
  return res.result || {}
}

async function withLlm<T>(
  fn: (llm: ReturnType<typeof llmPayload>, runtime: LlmRuntime) => Promise<T>,
): Promise<T> {
  const runtime = await requireTextLlm()
  return fn(llmPayload(runtime), runtime)
}

export async function aiRewrite(opts: {
  title: string
  body: string
  instruction?: string
}) {
  return withLlm(async (llm) => {
    const r = await aiRun('rewrite', { llm, ...opts })
    return String(r.body || '')
  })
}

export async function aiTitles(opts: { title: string; body: string; count?: number }) {
  return withLlm(async (llm) => {
    const r = await aiRun('titles', { llm, ...opts })
    return (Array.isArray(r.titles) ? r.titles : []).map(String)
  })
}

export async function aiTags(opts: {
  title: string
  body: string
  existing?: string[]
  count?: number
}) {
  return withLlm(async (llm) => {
    const r = await aiRun('tags', { llm, ...opts })
    return (Array.isArray(r.tags) ? r.tags : []).map(String)
  })
}

export async function aiGenerate(opts: {
  topic: string
  reference?: string
  instruction?: string
}) {
  return withLlm(async (llm) => {
    const r = await aiRun('generate', { llm, ...opts })
    return {
      title: String(r.title || opts.topic),
      body: String(r.body || ''),
      tags: (Array.isArray(r.tags) ? r.tags : []).map(String),
    }
  })
}

/** 自由文本补全（客服私信等，不套笔记 title/body/tags 结构） */
export async function aiComplete(opts: { system?: string; user: string }) {
  return withLlm(async (llm) => {
    const r = await aiRun('complete', {
      llm,
      system: opts.system || '',
      user: opts.user,
    })
    return String(r.text || r.body || '').trim()
  })
}

/**
 * 仪表盘「今日最火」：根据用途备注 + 行业词，让 LLM 产出更适合搜热帖的关键词。
 * 只返回一行短查询词，失败时回退 industryKeyword。
 */
export async function aiHotSearchQuery(opts: {
  remark?: string
  industryKeyword: string
}) {
  const industry = opts.industryKeyword.trim()
  const remark = (opts.remark || '').trim()
  if (!industry && !remark) return ''

  try {
    const text = await aiComplete({
      system:
        '你是小红书内容运营助手。根据用户给的用途备注与行业信息，产出1个最适合在小红书「按最热」搜索的短关键词。' +
        '要求：中文；2～12个字为宜；具体、可搜、偏内容选题；不要标点、不要引号、不要解释、不要多行；只输出关键词本身。',
      user: [
        remark ? `用途备注：${remark}` : '',
        industry ? `行业/方向：${industry}` : '',
        '请给出搜索关键词：',
      ]
        .filter(Boolean)
        .join('\n'),
    })
    const line = text
      .split(/\n/)[0]
      ?.replace(/^["'「」《》\s]+|["'「」《》\s.。！？]+$/g, '')
      .trim()
    if (line && line.length <= 24) return line
  } catch {
    /* fall through */
  }
  return industry || remark
}

export async function aiCompetitorChat(opts: {
  message: string
  context: unknown
  history?: { role: string; content: string }[]
}) {
  return withLlm(async (llm) => {
    const r = await aiRun(
      'competitor-chat',
      {
        llm,
        message: opts.message,
        context: opts.context,
        history: opts.history || [],
      },
      180,
    )
    return String(r.reply || '')
  })
}

export type ViralAnalyzeItem = {
  title?: string
  authorName?: string
  likes?: number
  collects?: number
  comments?: number
  shares?: number
  desc?: string
}

export async function aiViralAnalyze(opts: {
  scope: 'item' | 'group'
  platform?: string
  items: ViralAnalyzeItem[]
}) {
  return withLlm(async (llm) => {
    const r = await aiRun(
      'viral-analyze',
      {
        llm,
        scope: opts.scope,
        platform: opts.platform || 'xhs',
        items: opts.items,
      },
      180,
    )
    return {
      summary: String(r.summary || '').trim(),
      hooks: (Array.isArray(r.hooks) ? r.hooks : []).map(String),
      structure: String(r.structure || '').trim(),
      suggestions: (Array.isArray(r.suggestions) ? r.suggestions : []).map(String),
      score: Math.max(0, Math.min(100, Number(r.score) || 0)),
    }
  })
}

export async function aiGenerateImage(opts: {
  prompt: string
  size?: string
  quality?: string
  draftId?: string
  platform?: string
  /** 默认入库到素材中心 */
  persist?: boolean
}) {
  const runtime = await requireImageLlm()
  const llm = llmPayload(runtime)
  const timeoutSecs = Math.max(60, runtime.timeoutSec || 180)
  const r = await aiRun(
    'image-generate',
    {
      llm: { ...llm, timeoutSec: timeoutSecs },
      prompt: opts.prompt,
      size: opts.size || '1024x1024',
      quality: opts.quality || 'medium',
    },
    timeoutSecs,
  )
  const url = String(r.url || '')
  if (!url) throw new Error('未返回图片地址')
  const model = String(r.model || runtime.model)
  if (opts.persist === false) return { url, model }

  const { assetIngest, mediaDisplaySrc } = await import('@/api/assets')
  const asset = await assetIngest({
    sourceUrl: url,
    kind: 'image',
    source: 'ai-generate',
    prompt: opts.prompt,
    model,
    draftId: opts.draftId,
    platform: opts.platform,
  })
  return {
    url: mediaDisplaySrc(asset.absPath),
    path: asset.absPath,
    assetId: asset.id,
    model,
  }
}

/** 优化文案改写指令 / 文生图提示词 / 图生图编辑指令 */
export async function aiOptimizePrompt(opts: {
  kind: 'copy' | 'image-generate' | 'image-edit'
  prompt: string
  styleHint?: string
  title?: string
  body?: string
  imageUrl?: string
  imageBase64?: string
  imageMime?: string
}) {
  return withLlm(async (llm) => {
    const r = await aiRun(
      'optimize-prompt',
      {
        llm,
        kind: opts.kind,
        prompt: opts.prompt,
        styleHint: opts.styleHint || '',
        title: opts.title || '',
        body: opts.body || '',
        imageUrl: opts.imageUrl,
        imageBase64: opts.imageBase64,
        imageMime: opts.imageMime,
      },
      120,
    )
    const prompt = String(r.prompt || '').trim()
    if (!prompt) throw new Error('优化结果为空')
    return { prompt, usedVision: Boolean(r.usedVision) }
  })
}

/** 图生图：调用 Images API /v1/images/edits */
export async function aiEditImage(opts: {
  prompt: string
  imageUrl?: string
  imageBase64?: string
  imageMime?: string
  size?: string
  quality?: string
  draftId?: string
  platform?: string
  persist?: boolean
}) {
  const runtime = await requireImageLlm()
  const llm = llmPayload(runtime)
  const timeoutSecs = Math.max(60, runtime.timeoutSec || 180)
  if (!opts.imageUrl && !opts.imageBase64) {
    throw new Error('图生图需要底图')
  }
  const r = await aiRun(
    'image-edit',
    {
      llm: { ...llm, timeoutSec: timeoutSecs },
      prompt: opts.prompt,
      imageUrl: opts.imageUrl,
      imageBase64: opts.imageBase64,
      imageMime: opts.imageMime,
      size: opts.size || '1024x1024',
      quality: opts.quality || 'medium',
    },
    timeoutSecs,
  )
  const url = String(r.url || '')
  if (!url) throw new Error('未返回图片地址')
  const model = String(r.model || runtime.model)
  if (opts.persist === false) return { url, model }

  const { assetIngest, mediaDisplaySrc } = await import('@/api/assets')
  const asset = await assetIngest({
    sourceUrl: url,
    kind: 'image',
    source: 'ai-edit',
    prompt: opts.prompt,
    model,
    draftId: opts.draftId,
    platform: opts.platform,
  })
  return {
    url: mediaDisplaySrc(asset.absPath),
    path: asset.absPath,
    assetId: asset.id,
    model,
  }
}
