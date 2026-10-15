import { ref } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { ElMessage } from 'element-plus'
import { aiComplete } from '@/api/ai'
import {
  clickXhsConvItem,
  probeXhsImState,
  probeXhsMessageBadge,
  readXhsUnreadMessages,
  sendXhsChatReply,
  withAiBrowserBanner,
  xhsAcqRun,
  type XhsAcqRunResult,
  type XhsChatMessage,
  type XhsImStateProbe,
} from '@/api/cdp'
import {
  autoOpsLogAppend,
  autoOpsLogList,
  autoOpsUpsertBinding,
  csMessageAppend,
  kbSearch,
  type AutoOpsConsoleLog,
} from '@/api/knowledge'

const profileId = ref('')
const enabled = ref(false)
const systemPrompt = ref(
  '你是小红书店铺客服。简洁礼貌、像真人客服。优先依据参考资料作答；没有参考资料时用你的常识与专业知识直接回答，不要说「知识库没有」「资料未收录」之类推脱话，也禁止在回复里提及「知识库」。不确定时可简短提醒以专业意见为准，或礼貌追问细节。',
)
const selectedKbIds = ref<string[]>([])
const intervalSec = ref(8)
const polling = ref(false)

/** 控制台日志色调：对齐 RPA 实时执行台 */
export type AutoOpsLogTone = 'neutral' | 'step' | 'cyan' | 'gold' | 'purple' | 'ok' | 'warn' | 'err'

export type AutoOpsLogEntry = {
  id: string
  time: string
  tag: string
  tone: AutoOpsLogTone
  message: string
}

let logSeq = 0
const logs = ref<AutoOpsLogEntry[]>([])
const lastBadge = ref<number | null>(null)
const cdpOnline = ref(false)

/** 本轮客服值守统计（侧栏「本次结果」） */
export type CsRunStats = {
  repliesSent: number
  sessionsTouched: number
  lastSummary: string
}
const csRunStats = ref<CsRunStats>({
  repliesSent: 0,
  sessionsTouched: 0,
  lastSummary: '',
})

function noteCsReplySent(peer: string, reply: string) {
  csRunStats.value = {
    ...csRunStats.value,
    repliesSent: csRunStats.value.repliesSent + 1,
    lastSummary: `已回复「${peer}」: ${reply.slice(0, 36)}`,
  }
}

function noteCsSessionTouch(peer: string) {
  const c = ctl()
  const key = (peer || '').trim() || '用户'
  if (c.touchedPeers.has(key)) return
  c.touchedPeers.add(key)
  csRunStats.value = {
    ...csRunStats.value,
    sessionsTouched: c.touchedPeers.size,
    lastSummary: csRunStats.value.lastSummary || `触及会话「${key}」`,
  }
}

function toLogTone(raw: string | undefined): AutoOpsLogTone {
  const t = (raw || 'neutral') as AutoOpsLogTone
  const ok: AutoOpsLogTone[] = ['neutral', 'step', 'cyan', 'gold', 'purple', 'ok', 'warn', 'err']
  return ok.includes(t) ? t : 'neutral'
}

function mapDbLog(row: AutoOpsConsoleLog): AutoOpsLogEntry {
  return {
    id: row.id,
    time: row.timeLabel || new Date(row.createdAt).toLocaleTimeString('zh-CN', { hour12: false }),
    tag: row.tag || '系统',
    tone: toLogTone(row.tone),
    message: row.message,
  }
}

function persistConsoleLog(
  channel: 'cs' | 'acq',
  entry: Pick<AutoOpsLogEntry, 'time' | 'tag' | 'tone' | 'message'>,
) {
  const pid = profileId.value
  if (!pid) return
  void autoOpsLogAppend({
    profileId: pid,
    channel,
    tag: entry.tag,
    tone: entry.tone,
    message: entry.message,
    timeLabel: entry.time,
  }).catch(() => {
    /* 落库失败不阻断界面 */
  })
}

/** 对方连续发消息时，静默等待多久再统一处理（毫秒） */
const BURST_QUIET_MS = 4500
/** 最长等待，避免对方一直刷屏永不回复 */
const BURST_MAX_WAIT_MS = 16000

/**
 * 轮询控制放 globalThis，避免 Vite HMR 换模块后旧 setInterval 仍在跑 → 同一句回两次。
 */
type PollCtl = {
  timer: ReturnType<typeof setInterval> | null
  pollGen: number
  tickRunning: boolean
  sendingFp: string
  seenFp: string
  baselineFp: string
  /** 已回复过的用户原文（不绑 peer，防昵称抖动） */
  repliedTails: Set<string>
  /** 正在处理的用户原文 */
  sendingTails: Set<string>
  pendingBurstFp: string
  pendingBurstSince: number
  waitingLoggedFp: string
  /** 本轮触及过的会话昵称 */
  touchedPeers: Set<string>
}

const POLL_KEY = '__xhsAutoOpsPollCtl_v3__'
function ctl(): PollCtl {
  const g = globalThis as unknown as Record<string, PollCtl>
  if (!g[POLL_KEY]) {
    g[POLL_KEY] = {
      timer: null,
      pollGen: 0,
      tickRunning: false,
      sendingFp: '',
      seenFp: '',
      baselineFp: '',
      repliedTails: new Set(),
      sendingTails: new Set(),
      pendingBurstFp: '',
      pendingBurstSince: 0,
      waitingLoggedFp: '',
      touchedPeers: new Set(),
    }
  }
  if (!(g[POLL_KEY] as PollCtl).touchedPeers) {
    ;(g[POLL_KEY] as PollCtl).touchedPeers = new Set()
  }
  return g[POLL_KEY]
}

function pushLog(message: string, tag = '系统', tone: AutoOpsLogTone = 'neutral') {
  const time = new Date().toLocaleTimeString('zh-CN', { hour12: false })
  const entry: AutoOpsLogEntry = {
    id: `local_cs_${Date.now()}_${++logSeq}`,
    time,
    tag,
    tone,
    message,
  }
  logs.value = [...logs.value, entry].slice(-200)
  persistConsoleLog('cs', entry)
}

/** 短闲聊/无独立问句 → 强制汇总成一条，避免模型对「1」「你好」各回一条 */
function shouldForceSummary(burst: string[]): boolean {
  if (burst.length <= 1) return true
  const ask = /[？?吗麼呢]|怎么|如何|多少|哪|什么|能否|可以吃|是不是/
  return !burst.some((t) => t.length >= 4 && ask.test(t))
}

/** 覆盖旧版「只按知识库、无依据勿编造」类系统词，避免继续推脱 */
function effectiveSystemPrompt() {
  const base = (systemPrompt.value || '').trim()
  const guard =
    '对外回复时：优先用参考资料；没有资料就用专业常识直接答。禁止提及「知识库/资料库」，禁止说「没有相关说明/未收录」等推脱话。'
  if (/只根据知识库|无依据时不要编造|知识库命中/.test(base)) {
    return `你是小红书店铺客服。简洁礼貌。${guard}`
  }
  return `${base}\n${guard}`
}

/** 发出前再挡一层「知识库」类话术 */
function sanitizeCustomerReply(raw: string) {
  let s = String(raw || '').trim()
  if (!s) return s
  s = s
    .replace(/抱歉[，,]?\s*知识库里没有[^。！？\n]*[。！？]?/g, '')
    .replace(/知识库[里中]?没有[^。！？\n]*[。！？]?/g, '')
    .replace(/资料[库里中]?未收录[^。！？\n]*[。！？]?/g, '')
    .replace(/不敢随意下结论哦?[。！？]?/g, '')
    .replace(/根据知识库[，,]?/g, '')
    .replace(/【?知识库】?/g, '')
    .replace(/\s{2,}/g, ' ')
    .trim()
  return s || raw.trim()
}

if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    const c = ctl()
    c.pollGen += 1
    if (c.timer) {
      clearInterval(c.timer)
      c.timer = null
    }
    polling.value = false
    c.tickRunning = false
    c.sendingFp = ''
    c.sendingTails.clear()
  })
}

function sleep(ms: number) {
  return new Promise<void>((r) => setTimeout(r, ms))
}

async function persistBinding() {
  if (!profileId.value) return
  await autoOpsUpsertBinding({
    profileId: profileId.value,
    enabled: enabled.value,
    systemPrompt: systemPrompt.value,
    kbBaseIdsJson: JSON.stringify(selectedKbIds.value),
  })
}

function stopPoll(silent = false) {
  const c = ctl()
  c.pollGen += 1
  polling.value = false
  if (c.timer) {
    clearInterval(c.timer)
    c.timer = null
  }
  c.pendingBurstFp = ''
  c.pendingBurstSince = 0
  c.waitingLoggedFp = ''
  c.sendingFp = ''
  c.sendingTails.clear()
  if (!silent) pushLog('已停止轮询，沙箱待命', '执行完毕', 'ok')
}

/** 取「上次己方发送之后」对方连续发来的消息（尾部 burst） */
function collectRecvBurst(messages: XhsChatMessage[] | undefined): string[] {
  const msgs = messages || []
  let lastSent = -1
  for (let i = 0; i < msgs.length; i++) {
    if (msgs[i].direction === 'sent') lastSent = i
  }
  const burst: string[] = []
  for (let i = lastSent + 1; i < msgs.length; i++) {
    const m = msgs[i]
    const t = (m.text || '').trim()
    if (!t) continue
    if (m.direction === 'recv' || m.direction === 'unknown') burst.push(t)
  }
  // 去重相邻完全相同内容（偶发重复 DOM）
  return burst.filter((t, i) => i === 0 || t !== burst[i - 1])
}

function burstFingerprint(peer: string, burst: string[]): string {
  return `burst:${peer}:${burst.join('||')}`
}

function lastIncomingText(state: XhsImStateProbe): string {
  const burst = collectRecvBurst(state.messages)
  if (burst.length) return burst[burst.length - 1]
  return (state.lastText || '').trim()
}

type PendingBurst = {
  peer: string
  burst: string[]
  fp: string
}

/** 当前对话框是否有待回复的对方消息串 */
function openChatPending(state: XhsImStateProbe): PendingBurst | null {
  if (!state.inChat) return null
  if (state.lastDirection === 'sent') return null
  if (state.lastDirection !== 'recv' && state.lastDirection !== 'unknown') return null
  const peer = state.peerName || state.activeConvName || '用户'
  let burst = collectRecvBurst(state.messages)
  if (!burst.length) {
    const one = lastIncomingText(state)
    if (one) burst = [one]
  }
  if (!burst.length) return null
  const c = ctl()
  const tail = burst[burst.length - 1]
  // 同一句用户原文正在处理或已回过 → 绝不二次触发（不依赖 peer / DOM sent 状态）
  if (c.repliedTails.has(tail) || c.sendingTails.has(tail)) return null
  const fp = burstFingerprint(peer, burst)
  if (fp === c.seenFp || fp === c.baselineFp || fp === c.sendingFp) return null
  return { peer, burst, fp }
}

/**
 * 连续消息防抖：对方还在连发时先等一等；稳定后再处理。
 * 返回 true = 可以处理；false = 继续攒。
 */
function waitForBurstSettle(fp: string, count: number): boolean {
  const c = ctl()
  const now = Date.now()
  if (fp !== c.pendingBurstFp) {
    c.pendingBurstFp = fp
    c.pendingBurstSince = now
    c.waitingLoggedFp = ''
    if (count > 1) {
      pushLog(`检测到对方连续 ${count} 条，等待是否还有新消息…`, '攒批等待', 'gold')
    }
    return false
  }
  const waited = now - c.pendingBurstSince
  if (waited < BURST_QUIET_MS && waited < BURST_MAX_WAIT_MS) {
    if (c.waitingLoggedFp !== fp) {
      c.waitingLoggedFp = fp
      pushLog(
        `已攒 ${count} 条，静默 ${Math.ceil((BURST_QUIET_MS - waited) / 1000)}s 后统一判断回复策略`,
        '攒批等待',
        'gold',
      )
    }
    return false
  }
  if (waited >= BURST_MAX_WAIT_MS && c.waitingLoggedFp !== `force:${fp}`) {
    c.waitingLoggedFp = `force:${fp}`
    pushLog(`已达最长等待，开始处理这 ${count} 条`, '步骤 2/5', 'step')
  }
  return true
}

function parseReplyPlan(raw: string, burstLen: number): { strategy: 'summary' | 'each'; replies: string[] } {
  const text = String(raw || '').trim()
  // 尝试从 markdown / 杂文中抠 JSON
  const fence = text.match(/```(?:json)?\s*([\s\S]*?)```/i)
  const candidate = fence?.[1]?.trim() || text
  const start = candidate.indexOf('{')
  const end = candidate.lastIndexOf('}')
  if (start >= 0 && end > start) {
    try {
      const obj = JSON.parse(candidate.slice(start, end + 1)) as {
        strategy?: string
        mode?: string
        replies?: unknown
        reply?: unknown
      }
      const strategyRaw = String(obj.strategy || obj.mode || '').toLowerCase()
      const strategy: 'summary' | 'each' =
        strategyRaw === 'each' || strategyRaw === 'one_by_one' || strategyRaw === '逐条'
          ? 'each'
          : 'summary'
      let replies: string[] = []
      if (Array.isArray(obj.replies)) {
        replies = obj.replies.map((x) => String(x || '').trim()).filter(Boolean)
      } else if (obj.reply != null) {
        replies = [String(obj.reply).trim()].filter(Boolean)
      }
      if (replies.length) {
        if (strategy === 'summary') return { strategy: 'summary', replies: [replies[0]] }
        // each：条数尽量对齐；不够则用最后一条补，多则截断
        if (replies.length < burstLen && replies.length > 0) {
          const last = replies[replies.length - 1]
          while (replies.length < burstLen) replies.push(last)
        }
        return { strategy: 'each', replies: replies.slice(0, Math.max(burstLen, 1)) }
      }
    } catch {
      /* fall through */
    }
  }
  // 解析失败：整段当汇总回复
  const cleaned = text
    .replace(/^```[\s\S]*?```$/m, '')
    .replace(/^\s*\{[\s\S]*\}\s*$/m, '')
    .trim()
  return { strategy: 'summary', replies: [cleaned || text].filter(Boolean) }
}

async function buildKbBlock(query: string): Promise<string> {
  if (!selectedKbIds.value.length) return ''
  try {
    const hits = await kbSearch(selectedKbIds.value, query, 5)
    if (hits.length) {
      pushLog(`向量检索命中 ${hits.length} 条参考资料`, '步骤 3/5', 'cyan')
      return hits
        .map(
          (h, i) =>
            `[${i + 1}] Q: ${h.question}\nA: ${h.answer}${h.score != null ? ` (score=${h.score})` : ''}`,
        )
        .join('\n\n')
    }
    pushLog('知识库无命中，将用模型常识直接作答', '步骤 3/5', 'gold')
  } catch (e) {
    pushLog(`知识库检索失败: ${e instanceof Error ? e.message : String(e)}`, '检索异常', 'err')
  }
  return ''
}

async function recordChat(
  pid: string,
  peer: string,
  direction: 'recv' | 'sent',
  text: string,
  source = 'auto',
) {
  const t = text.trim()
  if (!t) return
  try {
    await csMessageAppend({ profileId: pid, peerName: peer, direction, text: t, source })
  } catch {
    /* ignore persist errors */
  }
}

async function craftAndSendBurst(pid: string, peer: string, burst: string[]) {
  for (const t of burst) {
    await recordChat(pid, peer, 'recv', t, 'auto')
  }
  const lines = burst.map((t, i) => `${i + 1}. ${t}`).join('\n')
  const kbBlock = await buildKbBlock(burst.join('\n'))

  const kbHint = kbBlock
    ? `【参考资料】（优先采用；可结合常识补充，但不要与资料冲突）\n${kbBlock}`
    : '【参考资料】暂无命中。请直接用你的专业知识与常识回答用户问题，给出实用建议。'
  const replyRules = [
    '请给出适合私信发送的短回复（不要 markdown，不要解释过程）。',
    '严禁在回复中出现「知识库」「资料库」「未收录」「没有相关说明」等字样或同义推脱。',
    '没有参考资料时照样正常答疑，不要让用户感觉你在甩锅。',
  ].join('\n')

  if (burst.length === 1) {
    const prompt = [
      kbHint,
      `【对话】对方「${peer}」说：${burst[0]}`,
      replyRules,
    ].join('\n\n')
    const rawReply = await aiComplete({ system: effectiveSystemPrompt(), user: prompt })
    const reply = sanitizeCustomerReply(rawReply)
    if (!reply) {
      pushLog('LLM 未生成回复', 'AI识别', 'err')
      return
    }
    const sent = await sendXhsChatReply(pid, reply.slice(0, 500))
    pushLog(
      sent.ok ? `已发送: ${reply.slice(0, 60)}` : `发送失败: ${sent.message}`,
      sent.ok ? '步骤 5/5' : '发送失败',
      sent.ok ? 'ok' : 'err',
    )
    if (sent.ok) {
      noteCsReplySent(peer, reply)
      await recordChat(pid, peer, 'sent', reply, 'auto')
    }
    return
  }

  const forceSummary = shouldForceSummary(burst)
  if (forceSummary) {
    pushLog(`连续 ${burst.length} 条偏闲聊/无独立问句 → 汇总成 1 条回复`, '步骤 4/5', 'purple')
  } else {
    pushLog(`模型判断 ${burst.length} 条消息的回复策略…`, 'AI识别', 'purple')
  }
  const prompt = [
    kbHint,
    `【对方「${peer}」连续发送的 ${burst.length} 条消息】\n${lines}`,
    replyRules,
    forceSummary
      ? [
          '这些消息没有多个独立问题，请只输出 1 条汇总私信回复。',
          '只输出 JSON：{"strategy":"summary","replies":["一条汇总回复"]}',
          '简洁口语化，不要编号前缀，不要 markdown。',
        ].join('\n')
      : [
          '请判断回复策略并生成可直接发送的私信文案：',
          '- summary（优先）：语义相关/寒暄/补充说明/短句拼凑 → 必须只回 1 条',
          '- each：仅当多条是互不相关的独立问题时才用，按顺序各回一条',
          '只输出 JSON，不要 markdown。格式：',
          '{"strategy":"summary"|"each","replies":["回复1","..."]}',
          'summary 时 replies 只能有 1 条；禁止对同一问题回两条。',
          '每条简洁口语化，不要编号前缀。',
        ].join('\n'),
  ].join('\n\n')

  let raw = ''
  try {
    raw = await aiComplete({ system: effectiveSystemPrompt(), user: prompt })
  } catch (e) {
    pushLog(`LLM 调用失败: ${e instanceof Error ? e.message : String(e)}`, 'AI识别', 'err')
    return
  }
  let plan = parseReplyPlan(raw, burst.length)
  if (!plan.replies.length) {
    pushLog(`LLM 未生成可用回复（原始: ${raw.slice(0, 80) || '空'}）`, 'AI识别', 'err')
    return
  }
  plan = {
    ...plan,
    replies: plan.replies.map((r) => sanitizeCustomerReply(r)).filter(Boolean),
  }
  if (!plan.replies.length) {
    pushLog('LLM 回复经清洗后为空', 'AI识别', 'err')
    return
  }
  if (forceSummary || plan.strategy === 'summary') {
    plan = { strategy: 'summary', replies: [plan.replies[0]] }
  }

  if (plan.strategy === 'summary' || plan.replies.length === 1) {
    pushLog(`策略：汇总回复（${burst.length} 条→1 条）`, '步骤 4/5', 'purple')
    const reply = plan.replies[0].slice(0, 500)
    const sent = await sendXhsChatReply(pid, reply)
    pushLog(
      sent.ok ? `已发送: ${reply.slice(0, 60)}` : `发送失败: ${sent.message}`,
      sent.ok ? '步骤 5/5' : '发送失败',
      sent.ok ? 'ok' : 'err',
    )
    if (sent.ok) {
      noteCsReplySent(peer, reply)
      await recordChat(pid, peer, 'sent', reply, 'auto')
    }
    return
  }

  pushLog(`策略：逐条回复（共 ${plan.replies.length} 条）`, '步骤 4/5', 'purple')
  for (let i = 0; i < plan.replies.length; i++) {
    if (!polling.value) break
    const reply = plan.replies[i].slice(0, 500)
    const sent = await sendXhsChatReply(pid, reply)
    pushLog(
      sent.ok
        ? `已发送(${i + 1}/${plan.replies.length}): ${reply.slice(0, 50)}`
        : `发送失败(${i + 1}): ${sent.message}`,
      sent.ok ? '步骤 5/5' : '发送失败',
      sent.ok ? 'ok' : 'err',
    )
    if (!sent.ok) break
    noteCsReplySent(peer, reply)
    await recordChat(pid, peer, 'sent', reply, 'auto')
    if (i < plan.replies.length - 1) await sleep(900)
  }
}

async function startPoll() {
  if (!profileId.value) {
    ElMessage.warning('请选择 Profile')
    return
  }
  if (!cdpOnline.value) {
    ElMessage.warning('CDP 离线，请先启动浏览器并打开小红书消息页')
    return
  }
  if (!enabled.value) {
    ElMessage.warning('请先打开「启用」开关')
    return
  }
  await persistBinding()
  const c = ctl()
  if (c.timer) {
    clearInterval(c.timer)
    c.timer = null
  }
  const gen = ++c.pollGen
  try {
    const st = await probeXhsImState(profileId.value)
    const peer = st.peerName || st.activeConvName || ''
    const burst = collectRecvBurst(st.messages)
    c.baselineFp = burst.length ? burstFingerprint(peer, burst) : st.fingerprint || ''
    c.seenFp = c.baselineFp
  } catch {
    c.baselineFp = ''
    c.seenFp = ''
  }
  c.pendingBurstFp = ''
  c.pendingBurstSince = 0
  c.waitingLoggedFp = ''
  c.sendingFp = ''
  c.sendingTails.clear()
  c.repliedTails.clear()
  c.touchedPeers.clear()
  c.tickRunning = false
  csRunStats.value = { repliesSent: 0, sessionsTouched: 0, lastSummary: '' }
  polling.value = true
  pushLog(
    '自动化沙箱唤醒 CDP 实例，开始轮询未读（含无角标会话；连续消息攒批后由模型决定汇总/逐条）…',
    '启动任务',
    'neutral',
  )
  await tick()
  if (gen !== ctl().pollGen || !polling.value) return
  const ms = Math.min(Math.max(3, intervalSec.value) * 1000, 3000)
  ctl().timer = setInterval(() => {
    const cur = ctl()
    if (gen !== cur.pollGen) {
      if (cur.timer) {
        clearInterval(cur.timer)
        cur.timer = null
      }
      return
    }
    void tick()
  }, ms)
}

async function handlePending(pid: string, pending: PendingBurst) {
  const c = ctl()
  if (!waitForBurstSettle(pending.fp, pending.burst.length)) return
  const tail = pending.burst[pending.burst.length - 1] || ''
  if (!tail) return
  if (c.repliedTails.has(tail) || c.sendingTails.has(tail)) return
  if (c.sendingFp === pending.fp || c.seenFp === pending.fp) return

  // 先占坑再调模型，防止并发 tick / 双定时器各回一条
  c.sendingTails.add(tail)
  for (const t of pending.burst) c.sendingTails.add(t)
  c.sendingFp = pending.fp
  c.seenFp = pending.fp
  c.pendingBurstFp = ''
  c.pendingBurstSince = 0
  c.waitingLoggedFp = ''
  const preview =
    pending.burst.length === 1
      ? pending.burst[0].slice(0, 80)
      : pending.burst.map((t, i) => `${i + 1}.${t.slice(0, 24)}`).join(' / ')
  pushLog(`处理 · ${pending.peer}: ${preview}`, '步骤 1/5', 'step')
  noteCsSessionTouch(pending.peer)
  try {
    await withAiBrowserBanner(pid, async () => {
      await craftAndSendBurst(pid, pending.peer, pending.burst)
    })
    for (const t of pending.burst) c.repliedTails.add(t)
    c.seenFp = `${pending.fp}|replied`
    pushLog(`已锁定用户句，避免重复回复: ${tail.slice(0, 40)}`, '执行完毕', 'ok')
  } finally {
    c.sendingFp = ''
    for (const t of pending.burst) c.sendingTails.delete(t)
  }
}

async function tick() {
  const c = ctl()
  if (!polling.value || c.tickRunning || !profileId.value) return
  c.tickRunning = true
  const pid = profileId.value
  try {
    const badge = await probeXhsMessageBadge(pid)
    lastBadge.value = badge.count
    const hasNavBadge = (badge.count ?? 0) > 0
    const state = await probeXhsImState(pid)

    const openNeed = openChatPending(state)
    if (openNeed) {
      await handlePending(pid, openNeed)
      return
    }

    if (c.pendingBurstFp && Date.now() - c.pendingBurstSince < BURST_MAX_WAIT_MS) {
      return
    }

    const unreadConvs = state.unreadConversations || []
    if (!hasNavBadge && !unreadConvs.length) return

    await withAiBrowserBanner(pid, async () => {
      if (hasNavBadge) {
        const unread = await readXhsUnreadMessages(pid)
        const peer = unread.peerName || '用户'
        let burst = collectRecvBurst(unread.messages)
        if (!burst.length) {
          const recv = (unread.messages || [])
            .filter((m) => m.direction === 'recv' || m.direction === 'unknown')
            .map((m) => m.text.trim())
            .filter(Boolean)
          if (recv.length) burst = [recv[recv.length - 1]]
          else if (unread.conversations?.[0]?.preview) burst = [unread.conversations[0].preview]
        }
        if (!burst.length) {
          pushLog('有未读但未抓到用户文本', '探测未读', 'warn')
          return
        }
        const tail = burst[burst.length - 1]
        if (ctl().repliedTails.has(tail) || ctl().sendingTails.has(tail)) return
        const fp = burstFingerprint(peer, burst)
        if (fp === ctl().seenFp) return
        waitForBurstSettle(fp, burst.length)
        pushLog(`未读 · ${peer}（${burst.length} 条），进入攒批`, '步骤 1/5', 'cyan')
        return
      }

      const target = unreadConvs.find((x) => !x.active) || unreadConvs[0]
      if (!target) return
      const clicked = await clickXhsConvItem(pid, target.index)
      if (!clicked.ok) {
        pushLog(`打开会话失败: ${clicked.message}`, '打开会话', 'err')
        return
      }
      const after = await probeXhsImState(pid)
      const peer = after.peerName || after.activeConvName || target.name || clicked.peerName || '用户'
      let burst = collectRecvBurst(after.messages)
      if (!burst.length) {
        const one =
          lastIncomingText(after) || (target.preview || '').trim() || (after.lastText || '').trim()
        if (one) burst = [one]
      }
      if (!burst.length) {
        pushLog('会话已打开但未抓到用户文本', '打开会话', 'warn')
        return
      }
      const tail = burst[burst.length - 1]
      if (ctl().repliedTails.has(tail) || ctl().sendingTails.has(tail)) return
      const fp = burstFingerprint(peer, burst)
      if (fp === ctl().seenFp) return
      waitForBurstSettle(fp, burst.length)
      pushLog(`会话未读 · ${peer}（${burst.length} 条），进入攒批`, '步骤 1/5', 'cyan')
    })
  } catch (e) {
    pushLog(`轮询异常: ${e instanceof Error ? e.message : String(e)}`, '系统异常', 'err')
  } finally {
    ctl().tickRunning = false
  }
}

function applyBinding(partial: {
  profileId?: string
  enabled?: boolean
  systemPrompt?: string
  selectedKbIds?: string[]
}) {
  if (partial.profileId !== undefined) profileId.value = partial.profileId
  if (partial.enabled !== undefined) enabled.value = partial.enabled
  if (partial.systemPrompt !== undefined && partial.systemPrompt) {
    systemPrompt.value = partial.systemPrompt
  }
  if (partial.selectedKbIds !== undefined) selectedKbIds.value = partial.selectedKbIds
}

function setCdpOnline(v: boolean) {
  cdpOnline.value = v
}

function setIntervalSec(v: number) {
  intervalSec.value = v
  const c = ctl()
  if (polling.value && c.timer) {
    clearInterval(c.timer)
    const gen = c.pollGen
    const ms = Math.min(Math.max(3, intervalSec.value) * 1000, 3000)
    c.timer = setInterval(() => {
      const cur = ctl()
      if (gen !== cur.pollGen) {
        if (cur.timer) {
          clearInterval(cur.timer)
          cur.timer = null
        }
        return
      }
      void tick()
    }, ms)
  }
}

/**
 * AI 获客状态必须挂在 globalThis：Vite HMR 会重载本模块，
 * 若 ref 在模块内而 listen 回调仍指向旧闭包，进度/结束事件进不了当前 UI，
 * 页面会一直卡在「获客执行中」。
 */
type AcqCtl = {
  running: ReturnType<typeof ref<boolean>>
  logs: ReturnType<typeof ref<AutoOpsLogEntry[]>>
  lastResult: ReturnType<typeof ref<XhsAcqRunResult | null>>
  logSeq: number
  progressUn: UnlistenFn | null
  finishedUn: UnlistenFn | null
  listening: boolean
}
const ACQ_KEY = '__xhsAcqCtl_v2__'
function acqCtl(): AcqCtl {
  const g = globalThis as unknown as Record<string, AcqCtl>
  if (!g[ACQ_KEY]) {
    g[ACQ_KEY] = {
      running: ref(false),
      logs: ref<AutoOpsLogEntry[]>([]),
      lastResult: ref<XhsAcqRunResult | null>(null),
      logSeq: 0,
      progressUn: null,
      finishedUn: null,
      listening: false,
    }
  }
  return g[ACQ_KEY]
}

const acqRunning = acqCtl().running
const acqLogs = acqCtl().logs
const acqLastResult = acqCtl().lastResult

function pushAcqLog(message: string, tag = '系统', tone: AutoOpsLogTone = 'neutral') {
  const c = acqCtl()
  const time = new Date().toLocaleTimeString('zh-CN', { hour12: false })
  const entry: AutoOpsLogEntry = {
    id: `local_acq_${Date.now()}_${++c.logSeq}`,
    time,
    tag,
    tone,
    message,
  }
  c.logs.value = [...c.logs.value, entry].slice(-200)
  persistConsoleLog('acq', entry)
}

async function loadConsoleLogs(pid?: string) {
  const id = (pid || profileId.value || '').trim()
  const acq = acqCtl()
  if (!id) {
    logs.value = []
    acq.logs.value = []
    return
  }
  try {
    const [csRows, acqRows] = await Promise.all([
      autoOpsLogList(id, 'cs', 200),
      autoOpsLogList(id, 'acq', 200),
    ])
    logs.value = (csRows || []).map(mapDbLog)
    acq.logs.value = (acqRows || []).map(mapDbLog)
  } catch {
    /* 加载失败保留内存日志 */
  }
}

function pushAcqStepLog(ok: boolean, name: string, detail: string) {
  let tone: AutoOpsLogTone = ok ? 'step' : 'err'
  let tag = name
  if (/search|list_notes|open_note|job_start/i.test(name)) tone = ok ? 'cyan' : 'err'
  else if (/sort/i.test(name)) tone = ok ? 'gold' : 'warn'
  else if (/reply|match|fill|click_reply/i.test(name)) tone = ok ? 'purple' : 'err'
  else if (/close|完毕|ok/i.test(name) && ok) tone = 'ok'
  else if (/scroll|wheel/i.test(name)) tone = ok ? 'step' : 'warn'
  if (ok && /执行完毕|已发送|探测成功|后台启动/i.test(detail)) tone = 'ok'
  pushAcqLog(detail, tag, tone)
}

async function ensureAcqListeners() {
  const c = acqCtl()
  if (c.listening) return
  c.listening = true
  try {
    // 回调只写 globalThis 单例，HMR 后仍更新当前 UI 绑定的同一批 ref
    c.progressUn = await listen<{ name: string; ok: boolean; detail: string }>(
      'xhs-acq-progress',
      (ev) => {
        const p = ev.payload
        if (!p?.name) return
        pushAcqStepLog(!!p.ok, p.name, p.detail || '')
      },
    )
    c.finishedUn = await listen<XhsAcqRunResult>('xhs-acq-finished', (ev) => {
      const r = ev.payload
      const s = acqCtl()
      s.running.value = false
      if (!r) return
      s.lastResult.value = r
      pushAcqLog(r.message || '获客结束', '执行完毕', r.ok ? 'ok' : 'warn')
      if (r.ok) ElMessage.success(r.message || '获客完成')
      else ElMessage.warning(r.message || '获客未完成')
    })
  } catch {
    c.listening = false
  }
}

export type AcqStartOpts = {
  profileId: string
  searchKeyword: string
  commentKeyword: string
  sortLabel: string
  replyText: string
  maxNotes: number
  maxReplies: number
  maxScrolls: number
}

async function startAcq(opts: AcqStartOpts) {
  if (acqRunning.value) {
    ElMessage.warning('获客任务已在后台运行')
    return
  }
  if (!opts.profileId) {
    ElMessage.warning('请选择 Profile')
    return
  }
  if (!cdpOnline.value) {
    ElMessage.warning('CDP 离线，请先启动浏览器')
    return
  }
  const searchKw = opts.searchKeyword.trim()
  const commentKw = opts.commentKeyword.trim()
  const reply = opts.replyText.trim()
  if (!searchKw) {
    ElMessage.warning('请填写笔记搜索关键词')
    return
  }
  if (!commentKw) {
    ElMessage.warning('请填写评论匹配关键词')
    return
  }
  if (!reply) {
    ElMessage.warning('请填写自定义回复')
    return
  }

  await ensureAcqListeners()
  const s = acqCtl()
  s.running.value = true
  s.lastResult.value = {
    ok: false,
    profileId: opts.profileId,
    tabId: '',
    keyword: searchKw,
    commentKeyword: commentKw,
    sortLabel: opts.sortLabel,
    notesOpened: 0,
    commentsMatched: 0,
    repliesSent: 0,
    steps: [],
    message: '执行中…',
    probedAt: Math.floor(Date.now() / 1000),
  }
  pushAcqLog(
    `开始获客（后台）· 搜索「${searchKw}」· 评论匹配「${commentKw}」· 排序「${opts.sortLabel}」· 下滑 ${opts.maxScrolls} 次`,
    '启动任务',
    'neutral',
  )
  try {
    // invoke 立即返回；CDP 在 Rust 线程继续跑，不阻塞主界面
    const started = await xhsAcqRun({
      profileId: opts.profileId,
      keyword: searchKw,
      commentKeyword: commentKw,
      sortLabel: opts.sortLabel,
      replyText: reply,
      maxNotes: opts.maxNotes,
      maxRepliesPerNote: opts.maxReplies,
      maxCommentScrolls: opts.maxScrolls,
    })
    pushAcqLog(started.message || '已后台启动', '后台任务', 'ok')
    ElMessage.success(started.message || '获客已在后台运行，可继续操作其他页面')
  } catch (e) {
    s.running.value = false
    const msg = e instanceof Error ? e.message : String(e)
    pushAcqLog(`启动失败: ${msg}`, '系统异常', 'err')
    ElMessage.error(msg)
  }
}

function stopAcqHint() {
  ElMessage.info(
    '获客在后台独立线程执行。若浏览器已停但按钮仍卡在「执行中」，请点「解除卡住」重置状态。',
  )
}

/** 浏览器已结束但 UI 仍显示执行中时，强制复位前端 + Rust 占用标记 */
async function resetAcqStuck() {
  const s = acqCtl()
  s.running.value = false
  if (s.lastResult.value?.message === '执行中…') {
    s.lastResult.value = {
      ...s.lastResult.value,
      message: '已手动解除卡住（若后台仍在跑，请关闭对应浏览器标签）',
      ok: false,
    }
  }
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('xhs_acq_force_idle')
  } catch {
    /* 旧二进制无此命令时仍复位前端 */
  }
  pushAcqLog('已手动解除「执行中」状态', '状态复位', 'warn')
  ElMessage.success('已解除卡住，可重新开始获客')
}

export function useXhsAutoOpsStore() {
  return {
    profileId,
    enabled,
    systemPrompt,
    selectedKbIds,
    intervalSec,
    polling,
    logs,
    lastBadge,
    csRunStats,
    cdpOnline,
    pushLog,
    persistBinding,
    startPoll,
    stopPoll,
    applyBinding,
    setCdpOnline,
    setIntervalSec,
    acqRunning,
    acqLogs,
    acqLastResult,
    pushAcqLog,
    pushAcqStepLog,
    startAcq,
    stopAcqHint,
    resetAcqStuck,
    loadConsoleLogs,
  }
}
