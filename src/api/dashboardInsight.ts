import { invoke } from '@tauri-apps/api/core'

export type DashboardInsightConfig = {
  platform: string
  remark: string
  industryKeyword: string
  /** 自动刷新间隔（分钟），默认 30，范围 15～1440 */
  intervalMinutes: number
  updatedAt: string
}

export type IndustryInsightRun = {
  id: number
  platform: string
  account: string
  keyword: string
  postsJson: string
  analysisJson: string
  status: string
  errorMessage: string
  createdAt: string
}

export type HotPost = {
  id?: string
  title?: string
  authorName?: string
  coverUrl?: string
  /** 本机落盘路径（note-media/industry-insight/…） */
  coverLocalPath?: string
  url?: string
  likes?: number
  collects?: number
  comments?: number
  shares?: number
  desc?: string
}

export type InsightAnalysis = {
  summary: string
  hooks: string[]
  structure: string
  suggestions: string[]
  score: number
}

export async function dashboardInsightConfigGet(platform: string) {
  return invoke<DashboardInsightConfig>('dashboard_insight_config_get', { platform })
}

export async function dashboardInsightConfigUpsert(body: {
  platform: string
  remark?: string
  industryKeyword?: string
  intervalMinutes?: number
}) {
  return invoke<DashboardInsightConfig>('dashboard_insight_config_upsert', { body })
}

export async function industryInsightRunAdd(body: {
  platform?: string
  account?: string
  keyword?: string
  postsJson?: string
  analysisJson?: string
  status?: string
  errorMessage?: string
}) {
  return invoke<IndustryInsightRun>('industry_insight_run_add', { body })
}

export async function industryInsightRunLatest(platform: string) {
  return invoke<IndustryInsightRun | null>('industry_insight_run_latest', { platform })
}

export function parseHotPosts(json: string): HotPost[] {
  try {
    const arr = JSON.parse(json || '[]')
    if (!Array.isArray(arr)) return []
    return arr.map((x) => {
      const r = (x && typeof x === 'object' ? x : {}) as Record<string, unknown>
      return {
        id: String(r.id || ''),
        title: String(r.title || ''),
        authorName: String(r.authorName || r.author_name || ''),
        coverUrl: String(r.coverUrl || r.cover_url || ''),
        coverLocalPath: String(r.coverLocalPath || r.cover_local_path || ''),
        url: String(r.url || ''),
        likes: Number(r.likes || 0),
        collects: Number(r.collects || 0),
        comments: Number(r.comments || 0),
        shares: Number(r.shares || 0),
        desc: String(r.desc || ''),
      }
    })
  } catch {
    return []
  }
}

export function parseAnalysis(json: string): InsightAnalysis | null {
  try {
    const r = JSON.parse(json || '{}') as Record<string, unknown>
    if (!r || typeof r !== 'object') return null
    return {
      summary: String(r.summary || '').trim(),
      hooks: (Array.isArray(r.hooks) ? r.hooks : []).map(String),
      structure: String(r.structure || '').trim(),
      suggestions: (Array.isArray(r.suggestions) ? r.suggestions : []).map(String),
      score: Math.max(0, Math.min(100, Number(r.score) || 0)),
    }
  } catch {
    return null
  }
}
