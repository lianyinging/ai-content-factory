import { invoke } from '@tauri-apps/api/core'

export type CompetitorRow = {
  id: string
  noteUrl: string
  noteId: string
  title: string
  authorName: string
  coverUrl: string
  coverLocalPath?: string
  likes: number
  collects: number
  comments: number
  shares: number
  intervalMinutes: number
  nextRunAt: string
  lastRefreshedAt: string
  lastError: string
  status: string
  analysisScore: number
  analysisGrade: string
  opportunityCount: number
  keywordsJson: string
  analysisJson: string
  chatJson: string
  createdAt: string
  updatedAt: string
}

export type SnapshotRow = {
  id: number
  competitorId: string
  likes: number
  collects: number
  comments: number
  shares: number
  engagement: number
  capturedAt: string
  /** live=真实刷新；backfill=补齐展示 */
  source?: string
}

export type WatchedUserRow = {
  id: string
  externalUserId: string
  nickname: string
  avatarUrl: string
  profileUrl: string
  noteCount: number
  lastSyncedAt: string
  intervalMinutes: number
  nextRunAt: string
  status: string
  lastError: string
  latestNotesJson: string
  unreadEvents: number
  createdAt: string
  updatedAt: string
}

export type WatchedEventRow = {
  id: string
  watchedUserId: string
  eventType: string
  title: string
  noteUrl: string
  noteId: string
  isRead: boolean
  createdAt: string
}

export type OpportunityRow = {
  id: string
  competitorId: string
  keyword: string
  commentText: string
  authorNickname: string
  status: string
  createdAt: string
}

export type CommentRow = {
  id: string
  competitorId: string
  userName: string
  content: string
  likeCount: number
  createdAt: string
}

export type RefreshLogRow = {
  id: number
  targetId: string
  status: string
  message: string
  createdAt: string
}

export async function competitorList() {
  return invoke<CompetitorRow[]>('competitor_list')
}

export async function competitorAdd(body: {
  noteUrl: string
  noteId?: string
  title?: string
  authorName?: string
  coverUrl?: string
  likes?: number
  collects?: number
  comments?: number
  shares?: number
  intervalMinutes?: number
}) {
  return invoke<CompetitorRow>('competitor_add', { body })
}

export async function competitorDelete(id: string) {
  return invoke('competitor_delete', { id })
}

export async function competitorRefresh(body: {
  id: string
  noteUrl?: string
  noteId?: string
  title?: string
  authorName?: string
  coverUrl?: string
  likes?: number
  collects?: number
  comments?: number
  shares?: number
  analysisJson?: string
  analysisScore?: number
  analysisGrade?: string
  lastError?: string
  commentsJson?: string
  /** 执行记录：手动刷新 / 后台监控 */
  trigger?: string
}) {
  return invoke<CompetitorRow>('competitor_refresh', { body })
}

export async function competitorSnapshots(competitorId: string, days = 30) {
  return invoke<SnapshotRow[]>('competitor_snapshots', { competitorId, days })
}

/** 为监控笔记补齐近 N 天缺失快照（线性爬升，source=backfill） */
export async function competitorSnapshotsBackfill(opts?: {
  days?: number
  competitorId?: string
}) {
  return invoke<{
    ok: boolean
    days: number
    competitorsTouched: number
    snapshotsFilled: number
  }>('competitor_snapshots_backfill', {
    days: opts?.days ?? 30,
    competitorId: opts?.competitorId ?? null,
  })
}

export async function competitorSetChat(id: string, chatJson: string) {
  await invoke('competitor_set_chat', { id, chatJson })
}

export async function competitorSetAnalysis(id: string, analysisJson: string) {
  await invoke('competitor_set_analysis', { id, analysisJson })
}

export async function competitorComments(competitorId: string) {
  return invoke<CommentRow[]>('competitor_comments', { competitorId })
}

export async function competitorRefreshLogs(targetId: string) {
  return invoke<RefreshLogRow[]>('competitor_refresh_logs', { targetId })
}

export async function competitorGetSettings() {
  return invoke<{ keywords: string[]; scanIntervalMinutes: number }>('competitor_get_settings')
}

export async function competitorPutSettings(body: {
  keywords?: string[]
  scanIntervalMinutes?: number
}) {
  return invoke<{ keywords: string[]; scanIntervalMinutes: number }>('competitor_put_settings', {
    keywords: body.keywords ?? null,
    scanIntervalMinutes: body.scanIntervalMinutes ?? null,
  })
}

export async function competitorScanOpportunities(competitorId: string) {
  return invoke<{ ok: boolean; created: number }>('competitor_scan_opportunities', {
    competitorId,
  })
}

export async function competitorListOpportunities(competitorId: string) {
  return invoke<OpportunityRow[]>('competitor_list_opportunities', { competitorId })
}

export async function competitorDeleteOpportunity(id: string) {
  await invoke('competitor_delete_opportunity', { id })
}

export async function competitorSetKeywords(id: string, keywords: string[]) {
  await invoke('competitor_set_keywords', { id, keywords })
}

export async function watchedUserList() {
  return invoke<WatchedUserRow[]>('watched_user_list')
}

export async function watchedUserAdd(body: {
  profileUrl: string
  externalUserId?: string
  nickname?: string
  avatarUrl?: string
}) {
  return invoke<WatchedUserRow>('watched_user_add', { body })
}

export async function watchedUserDelete(id: string) {
  await invoke('watched_user_delete', { id })
}

export async function watchedUserRefresh(body: {
  id: string
  nickname?: string
  avatarUrl?: string
  noteCount?: number
  latestNotesJson?: string
  lastError?: string
  newEvents?: {
    eventType?: string
    title?: string
    noteUrl?: string
    noteId?: string
  }[]
}) {
  return invoke<WatchedUserRow>('watched_user_refresh', { body })
}

export async function watchedUserEvents(watchedUserId: string) {
  return invoke<WatchedEventRow[]>('watched_user_events', { watchedUserId })
}

export async function watchedUserMarkRead(watchedUserId: string) {
  await invoke('watched_user_mark_read', { watchedUserId })
}

/** 规则评分（不调 LLM） */
export function computeAnalysis(row: CompetitorRow, snaps: SnapshotRow[]) {
  const engagement = row.likes + row.collects + row.comments + row.shares
  const first = snaps[0]
  const last = snaps[snaps.length - 1]
  let growth = 0
  if (first && last && first.engagement > 0) {
    growth = ((last.engagement - first.engagement) / Math.max(1, first.engagement)) * 100
  }
  let score = 50
  score += Math.min(25, Math.log10(Math.max(1, engagement)) * 8)
  score += Math.min(15, Math.max(-10, growth / 4))
  score = Math.round(Math.max(0, Math.min(100, score)))
  const grade = score >= 85 ? 'S' : score >= 70 ? 'A' : score >= 55 ? 'B' : score >= 40 ? 'C' : 'D'
  const suggestions: string[] = []
  if (row.collects > row.likes * 0.8) suggestions.push('收藏/点赞比偏高，适合做工具向、清单向内容')
  if (row.comments > 50) suggestions.push('评论活跃，可关注评论区高频问题做衍生笔记')
  if (growth > 20) suggestions.push('近期互动增速明显，建议拆解标题与封面结构')
  if (!suggestions.length) suggestions.push('持续观察指标与封面/标题变化，沉淀可复用结构')
  return {
    score,
    grade,
    engagement,
    growthPct: Math.round(growth * 10) / 10,
    summary: `综合评分 ${score}（${grade}），互动量 ${engagement}，区间增速 ${growth.toFixed(1)}%。`,
    suggestions,
  }
}
