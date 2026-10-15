import { catbusRun, parseCatbusError, reportCatbusError } from '@/api/catbus'
import {
  competitorRefresh,
  competitorSetAnalysis,
  competitorSnapshots,
  computeAnalysis,
  watchedUserRefresh,
  type CompetitorRow,
  type WatchedUserRow,
} from '@/api/competitors'
import { noteRefForCatbus } from '@/utils/xhsNoteRef'

function parseItem(env: Awaited<ReturnType<typeof catbusRun>>) {
  const data = env.data as Record<string, unknown> | undefined
  const item =
    (data?.item as Record<string, unknown> | undefined) ||
    (data?.note as Record<string, unknown> | undefined) ||
    data
  return item && typeof item === 'object' ? item : null
}

async function fetchCommentsJson(noteRef: string): Promise<string> {
  try {
    const env = await catbusRun(['xhs', 'comment', 'list', noteRef], { timeoutSecs: 90 })
    const data = env.data as Record<string, unknown> | undefined
    const list =
      (data?.comments as unknown[]) ||
      (data?.list as unknown[]) ||
      (Array.isArray(data) ? data : [])
    const mapped = (Array.isArray(list) ? list : []).slice(0, 60).map((raw, i) => {
      const c = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>
      const user = (c.user as Record<string, unknown>) || {}
      return {
        id: String(c.id || c.commentId || `c${i}`),
        userName: String(user.nickname || user.name || c.userName || ''),
        content: String(c.content || c.text || ''),
        likeCount: Number(c.likeCount ?? c.likes ?? 0),
      }
    })
    return JSON.stringify(mapped)
  } catch {
    return '[]'
  }
}

export type SyncResult = {
  ok: boolean
  risk?: boolean
  error?: string
  row?: CompetitorRow
}

/** 刷新单条笔记监控（供手动刷新与后台定时共用） */
export async function syncCompetitorNote(
  row: CompetitorRow,
  opts?: { reportError?: boolean; trigger?: string },
): Promise<SyncResult> {
  const report = opts?.reportError !== false
  const trigger = opts?.trigger || '后台监控'
  const noteRef = noteRefForCatbus(row.noteUrl, row.noteId)
  if (!noteRef) {
    const error = '无法解析笔记链接，请粘贴带 xsec_token 的完整 URL'
    if (report) await reportCatbusError(new Error(error), '获取详情失败')
    return { ok: false, error }
  }
  try {
    const env = await catbusRun(['xhs', 'item', 'get', noteRef], { timeoutSecs: 90 })
    if (env.ok === false || env.error) {
      const info = parseCatbusError(env, '获取详情失败')
      const risk =
        Boolean(info.verifyUrl) ||
        Boolean(info.riskKind) ||
        /RISK_CONTROL|人机验证|461|471/i.test(`${info.code} ${info.message} ${info.hint}`)
      if (report) await reportCatbusError(env, '获取详情失败')
      return { ok: false, risk, error: info.fullText }
    }
    const item = parseItem(env)
    if (!item) {
      const error = '获取详情失败：响应无笔记数据'
      if (report) await reportCatbusError(env, error)
      return { ok: false, error }
    }
    const stats = (item.stats as Record<string, number>) || {}
    const likes = Number(stats.likes ?? item.likes ?? 0)
    const collects = Number(stats.collects ?? item.collects ?? 0)
    const commentsN = Number(stats.comments ?? item.comments ?? 0)
    const shares = Number(stats.shares ?? item.shares ?? 0)
    const itemUrl = String(item.url || '')
    const nextNoteUrl = itemUrl && /xsec_token=/i.test(itemUrl) ? itemUrl : row.noteUrl
    const commentsJson = await fetchCommentsJson(
      noteRefForCatbus(nextNoteUrl, row.noteId || String(item.id || '')),
    )
    const draft: CompetitorRow = {
      ...row,
      noteUrl: nextNoteUrl,
      likes,
      collects,
      comments: commentsN,
      shares,
    }
    const snap = await competitorSnapshots(row.id, 15)
    const a = computeAnalysis(draft, [
      ...snap,
      {
        id: 0,
        competitorId: row.id,
        likes,
        collects,
        comments: commentsN,
        shares,
        engagement: likes + collects + commentsN + shares,
        capturedAt: new Date().toISOString(),
      },
    ])
    const updated = await competitorRefresh({
      id: row.id,
      noteUrl: nextNoteUrl,
      noteId: String(item.id || row.noteId),
      title: String(item.title || row.title),
      authorName: String(
        (item.author as { name?: string })?.name || item.authorName || row.authorName,
      ),
      coverUrl: String(item.coverUrl || item.cover || row.coverUrl),
      likes,
      collects,
      comments: commentsN,
      shares,
      analysisJson: JSON.stringify(a),
      analysisScore: a.score,
      analysisGrade: a.grade,
      commentsJson,
      lastError: '',
      trigger,
    })
    await competitorSetAnalysis(row.id, JSON.stringify(a))
    return { ok: true, row: updated }
  } catch (e) {
    const info = parseCatbusError(e, '刷新失败')
    const risk =
      Boolean(info.verifyUrl) ||
      /RISK_CONTROL|人机验证|461|471/i.test(`${info.code} ${info.message} ${info.hint}`)
    if (report) await reportCatbusError(e, '刷新失败')
    return { ok: false, risk, error: info.fullText }
  }
}

export async function syncWatchedUser(
  row: WatchedUserRow,
  opts?: { reportError?: boolean },
): Promise<SyncResult> {
  const report = opts?.reportError !== false
  const uid = String(row.externalUserId || '').trim()
  if (!uid) {
    const error = '无法解析用户 ID'
    if (report) await reportCatbusError(new Error(error), '同步用户失败')
    return { ok: false, error }
  }
  try {
    const env = await catbusRun(['xhs', 'user', 'get', uid], { timeoutSecs: 90 })
    if (env.ok === false || env.error) {
      const info = parseCatbusError(env, '同步用户失败')
      const risk =
        Boolean(info.verifyUrl) ||
        Boolean(info.riskKind) ||
        /RISK_CONTROL|人机验证|461|471/i.test(`${info.code} ${info.message} ${info.hint}`)
      if (report) await reportCatbusError(env, '同步用户失败')
      return { ok: false, risk, error: info.fullText }
    }
    const data = env.data as Record<string, unknown> | undefined
    const user =
      (data?.user as Record<string, unknown> | undefined) ||
      (data?.profile as Record<string, unknown> | undefined) ||
      data
    const notesRaw =
      (data?.notes as unknown[]) ||
      (data?.items as unknown[]) ||
      (user?.notes as unknown[]) ||
      []
    const notes = (Array.isArray(notesRaw) ? notesRaw : []).slice(0, 20).map((raw) => {
      const n = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>
      return {
        id: String(n.id || n.noteId || ''),
        title: String(n.title || ''),
        url: String(n.url || n.noteUrl || ''),
        likes: Number((n.stats as { likes?: number })?.likes ?? n.likes ?? 0),
      }
    })
    let prevIds: string[] = []
    try {
      prevIds = (JSON.parse(row.latestNotesJson || '[]') as { id?: string }[])
        .map((x) => String(x.id || ''))
        .filter(Boolean)
    } catch {
      prevIds = []
    }
    const newEvents = notes
      .filter((n) => n.id && !prevIds.includes(n.id))
      .slice(0, 10)
      .map((n) => ({
        eventType: 'new_note',
        title: n.title || '新笔记',
        noteUrl: n.url,
        noteId: n.id,
      }))
    await watchedUserRefresh({
      id: row.id,
      nickname: String(user?.nickname || user?.name || row.nickname),
      avatarUrl: String(user?.avatar || user?.avatarUrl || row.avatarUrl),
      noteCount: notes.length,
      latestNotesJson: JSON.stringify(notes),
      newEvents,
      lastError: '',
    })
    return { ok: true }
  } catch (e) {
    const info = parseCatbusError(e, '同步用户失败')
    const risk =
      Boolean(info.verifyUrl) ||
      /RISK_CONTROL|人机验证|461|471/i.test(`${info.code} ${info.message} ${info.hint}`)
    if (report) await reportCatbusError(e, '同步用户失败')
    return { ok: false, risk, error: info.fullText }
  }
}

/** SQLite datetime('now') 本地墙钟 / ISO → 是否已到期 */
export function isDueAt(nextRunAt: string | undefined | null, now = Date.now()): boolean {
  const s = String(nextRunAt || '').trim()
  if (!s) return true
  const m = s.match(/^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})(?::(\d{2}))?/)
  if (m) {
    const t = new Date(
      Number(m[1]),
      Number(m[2]) - 1,
      Number(m[3]),
      Number(m[4]),
      Number(m[5]),
      Number(m[6] || 0),
    ).getTime()
    return t <= now
  }
  const t = Date.parse(s)
  return Number.isNaN(t) ? true : t <= now
}
