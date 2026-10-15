export type Platform = 'xhs' | 'douyin'

export interface ContentItem {
  id: string
  platform: Platform
  title: string
  desc: string
  coverUrl: string
  imageUrls?: string[]
  authorName: string
  authorId: string
  likes: number
  comments: number
  collects: number
  shares: number
  url: string
  publishedAt: string
  noteType: string
  raw?: unknown
}

export interface LibraryRow {
  id: number
  platform: string
  itemId: string
  title: string
  authorName: string
  likes: number
  comments: number
  collects: number
  shares: number
  url: string
  coverUrl: string
  coverLocalPath?: string
  imagesJson?: string
  imagesLocalJson?: string
  desc: string
  publishedAt: string
  rawJson: string
  savedAt: string
  groupId?: string
  analysisJson?: string
  analyzedAt?: string
}

export interface LibraryGroup {
  id: string
  platform: string
  name: string
  sortOrder: number
  analysisJson: string
  analyzedAt: string
  createdAt: string
  updatedAt: string
  count: number
}

export interface LibraryGroupListResult {
  groups: LibraryGroup[]
  totalCount: number
  ungroupedCount: number
  ungroupedAnalysisJson: string
  ungroupedAnalyzedAt: string
}

export interface ViralAnalysis {
  summary: string
  hooks: string[]
  structure: string
  suggestions: string[]
  score: number
}

export interface DoctorInfo {
  ready: boolean
  node: string | null
  cli: string | null
  catbusHome: string
  pluginDir: string
  error: string | null
}

export interface CatbusEnvelope {
  ok?: boolean
  data?: unknown
  page?: unknown
  error?: {
    code?: string
    message?: string
    /** 小红书 461/471 时 hint 内含网页验证链接（见 catbus FAQ RISK_CONTROL） */
    hint?: string
    /** RISK_CONTROL 时多为 { kind: captcha|rate_limit|blocked, ... }，其它场景可能是字符串 */
    detail?: unknown
  }
  _meta?: {
    exitCode?: number
    catbusHome?: string
    stderr?: string
  }
  [key: string]: unknown
}
