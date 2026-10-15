import { invoke } from '@tauri-apps/api/core'

type KbJobResult = {
  ok: boolean
  result?: Record<string, unknown> | null
  error?: string | null
}

async function kbRun(action: string, payload: Record<string, unknown> = {}, timeoutSecs = 120) {
  const res = await invoke<KbJobResult>('kb_run', {
    req: { action, payload, timeoutSecs },
  })
  if (!res.ok) throw new Error(res.error || String(res.result?.error || `知识库 ${action} 失败`))
  return res.result || {}
}

export type KbCapability = {
  ok?: boolean
  available?: boolean
  message?: string
  embedding?: { available?: boolean; path?: string; message?: string; model?: string }
  chroma?: { available?: boolean; message?: string }
}

export type KbBase = {
  id: string
  name: string
  description?: string
  qaCount?: number
  embeddingModel?: string
}

export type KbQa = {
  id: string
  baseId: string
  question: string
  answer: string
  tags?: string[]
}

export type KbSearchHit = {
  id: string
  baseId?: string
  question: string
  answer: string
  tags?: string
  score?: number
}

export async function kbCapability() {
  return (await kbRun('capability', {}, 30)) as KbCapability
}

export async function kbListBases() {
  const r = await kbRun('list_bases', {}, 60)
  return (r.items as KbBase[]) || []
}

export async function kbCreateBase(name: string, description = '') {
  const r = await kbRun('create_base', { name, description }, 60)
  return r.item as KbBase
}

export async function kbDeleteBase(baseId: string) {
  await kbRun('delete_base', { baseId }, 60)
}

export async function kbListQa(baseId: string, page = 1, pageSize = 50, q = '') {
  const r = await kbRun('list_qa', { baseId, page, pageSize, q }, 60)
  return {
    items: (r.items as KbQa[]) || [],
    total: Number(r.total || 0),
    page: Number(r.page || page),
    pageSize: Number(r.pageSize || pageSize),
  }
}

export async function kbUpsertQa(body: {
  baseId: string
  qaId?: string
  question: string
  answer: string
  tags?: string[]
}) {
  const r = await kbRun('upsert_qa', body, 180)
  return r.item as KbQa
}

export async function kbDeleteQa(baseId: string, qaId: string) {
  await kbRun('delete_qa', { baseId, qaId }, 60)
}

export async function kbImportQa(baseId: string, rawText: string) {
  return kbRun('import_qa', { baseId, rawText }, 300)
}

export async function kbSearch(baseIds: string[], query: string, topK = 5) {
  const r = await kbRun('search', { baseIds, query, topK }, 180)
  return (r.items as KbSearchHit[]) || []
}

export type AutoOpsBinding = {
  profileId: string
  enabled: boolean
  systemPrompt: string
  kbBaseIdsJson: string
  updatedAt: number
}

export async function autoOpsGetBinding(profileId: string) {
  return invoke<AutoOpsBinding | null>('auto_ops_get_binding', { profileId })
}

export async function autoOpsUpsertBinding(body: {
  profileId: string
  enabled: boolean
  systemPrompt: string
  kbBaseIdsJson: string
  updatedAt?: number
}) {
  return invoke<AutoOpsBinding>('auto_ops_upsert_binding', {
    body: { ...body, updatedAt: body.updatedAt ?? 0 },
  })
}

export type AutoOpsConsoleLog = {
  id: string
  profileId: string
  channel: 'cs' | 'acq' | string
  tag: string
  tone: string
  message: string
  timeLabel: string
  createdAt: number
}

export async function autoOpsLogAppend(body: {
  profileId: string
  channel: 'cs' | 'acq'
  tag?: string
  tone?: string
  message: string
  timeLabel?: string
}) {
  return invoke<AutoOpsConsoleLog>('auto_ops_log_append', { body })
}

export async function autoOpsLogList(
  profileId: string,
  channel: 'cs' | 'acq',
  limit = 200,
) {
  return invoke<AutoOpsConsoleLog[]>('auto_ops_log_list', { profileId, channel, limit })
}

export type AcqReplyHistoryRow = {
  id: string
  profileId: string
  searchKeyword: string
  commentKeyword: string
  authorName: string
  commentText: string
  replyText: string
  noteTitle: string
  noteIndex: number
  ok: boolean
  createdAt: number
}

export async function acqReplyHistoryList(profileId: string, limit = 100) {
  return invoke<AcqReplyHistoryRow[]>('acq_reply_history_list', { profileId, limit })
}

export async function acqReplyHistoryClear(profileId: string) {
  return invoke<{ ok: boolean; deleted: number }>('acq_reply_history_clear', { profileId })
}

export type CsSession = {
  id: string
  profileId: string
  peerName: string
  lastPreview: string
  messageCount: number
  unreadBadge: number
  lastDirection: string
  updatedAt: number
  createdAt: number
}

export type CsMessage = {
  id: string
  sessionId: string
  profileId: string
  direction: 'recv' | 'sent' | string
  text: string
  source: string
  createdAt: number
}

export async function csSessionList(profileId: string) {
  return invoke<CsSession[]>('cs_session_list', { profileId })
}

export async function csMessageList(sessionId: string, limit = 300) {
  return invoke<CsMessage[]>('cs_message_list', { sessionId, limit })
}

export async function csMessageAppend(body: {
  profileId: string
  peerName: string
  direction: 'recv' | 'sent' | string
  text: string
  source?: string
}) {
  return invoke<CsMessage>('cs_message_append', { body })
}

export async function csMessagesSync(body: {
  profileId: string
  peerName: string
  messages: { direction: string; text: string }[]
  unreadBadge?: number
}) {
  return invoke<CsSession>('cs_messages_sync', { body })
}

export async function csSessionTouch(body: {
  profileId: string
  peerName: string
  lastPreview?: string
  unreadBadge?: number
}) {
  return invoke<CsSession>('cs_session_touch', { body })
}
