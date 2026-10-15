<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import * as echarts from 'echarts'
import { ElMessage, ElMessageBox } from 'element-plus'
import { CopyDocument, Refresh, Setting } from '@element-plus/icons-vue'
import { authList, authStatus } from '@/api/catbus'
import {
  browserCdpStatusAll,
  listBrowserProfiles,
  parseProfileSession,
  type BrowserProfile,
  type BrowserStatus,
} from '@/api/cdp'
import {
  competitorList,
  competitorSnapshots,
  competitorSnapshotsBackfill,
  watchedUserList,
  type CompetitorRow,
  type SnapshotRow,
} from '@/api/competitors'
import { dashboardInsightConfigUpsert, type HotPost } from '@/api/dashboardInsight'
import { mediaDisplaySrc } from '@/api/assets'
import { useIndustryInsightStore } from '@/state/industryInsight'
import { useLibraryAnalyzeStore } from '@/state/libraryAnalyze'
import { useImageJobsStore } from '@/state/imageJobs'
import { isPublishJobRunning } from '@/state/publishCdp'

const router = useRouter()
const PLATFORM = 'xhs'

const insight = useIndustryInsightStore()
const {
  running: insightRunning,
  phase: insightPhase,
  lastError: insightLastError,
  lastRun: insightLastRun,
  posts: insightPosts,
  analysis: insightAnalysis,
  config: insightConfig,
  searchAccount: insightAccount,
  searchNickname: insightNickname,
  searchQuery: insightSearchQuery,
  statusText: insightStatus,
  refreshNow: insightRefresh,
  loadFromDb: insightLoad,
  setConfigLocal,
} = insight
const { hasActive: analyzeBusy, activeTitle: analyzeTitle } = useLibraryAnalyzeStore()
const { hasActive: imageBusy, refresh: refreshImageJobs } = useImageJobsStore()

type PcAccount = {
  account: string
  nickname: string
  loggedIn: boolean
  avatarUrl: string
  externalUserId: string
}

type CdpRow = {
  id: string
  name: string
  cdpOnline: boolean
  loggedIn: boolean
  nickname: string
  avatarUrl: string
  followers: string
  following: string
  likesCollect: string
  redId: string
}

const healthBusy = ref(false)
const pcAccounts = ref<PcAccount[]>([])
const cdpRows = ref<CdpRow[]>([])

const remarkDraft = ref('')
const keywordDraft = ref('')
const intervalDraft = ref(30)
const configSaving = ref(false)
const configDialogOpen = ref(false)

const insightIntervalMin = computed(() => {
  const n = Math.round(Number(insightConfig.value?.intervalMinutes))
  if (!Number.isFinite(n)) return 30
  return Math.min(1440, Math.max(15, n))
})

function openConfigDialog() {
  remarkDraft.value = insightConfig.value?.remark || ''
  keywordDraft.value = insightConfig.value?.industryKeyword || ''
  intervalDraft.value = insightIntervalMin.value
  configDialogOpen.value = true
}

const competitors = ref<CompetitorRow[]>([])
const watchedCount = ref(0)
const unreadEvents = ref(0)
const growthRows = ref<
  { id: string; title: string; authorName: string; dailyDelta: number; engagement: number }[]
>([])
const selectedNoteId = ref('')
/** total=全部监控合计；note=单笔记 */
const chartMode = ref<'total' | 'note'>('total')
const chartSnaps = ref<SnapshotRow[]>([])
const chartBusy = ref(false)
/** 加载竞品时缓存快照，合计图复用 */
const snapsByCompetitor = ref<Record<string, SnapshotRow[]>>({})

const healthChartEl = ref<HTMLElement | null>(null)
const healthCardsEl = ref<HTMLElement | null>(null)
const trendChartEl = ref<HTMLElement | null>(null)
const healthLogEl = ref<HTMLElement | null>(null)
let healthChart: echarts.ECharts | null = null
let trendChart: echarts.ECharts | null = null
let healthCardsRo: ResizeObserver | null = null
/** 增速折线窗口与赛跑动画（line-race）：平滑插值爬升 → 停 10s → 再播 */
const TREND_DAYS = 30
/** 平滑优先：日间密采样 + 略放慢 */
const TREND_RACE_MS = 10_000
const TREND_HOLD_MS = 10_000
/** 相邻两日之间的插值段数（越大越细越顺） */
const TREND_RACE_SUBSTEPS = 24
let trendRaceRaf = 0
let trendHoldTimer: ReturnType<typeof setTimeout> | null = null

function stopTrendClimbLoop() {
  if (trendRaceRaf) {
    cancelAnimationFrame(trendRaceRaf)
    trendRaceRaf = 0
  }
  if (trendHoldTimer) {
    clearTimeout(trendHoldTimer)
    trendHoldTimer = null
  }
}

/** 更匀速的缓动，减少两端突然加速感 */
function easeInOutSine(t: number) {
  return -(Math.cos(Math.PI * t) - 1) / 2
}

function lerp(a: number, b: number, t: number) {
  return a + (b - a) * t
}

type TrendDensePoint = {
  x: number
  likes: number
  collects: number
  comments: number
  shares: number
  dayLabel: string
}

/** 把按日快照展开成密采样折线点，供赛跑逐步揭示 */
function buildTrendDensePath(snaps: SnapshotRow[]): TrendDensePoint[] {
  const n = snaps.length
  if (!n) return []
  if (n === 1) {
    const s = snaps[0]!
    return [
      {
        x: 0,
        likes: s.likes,
        collects: s.collects,
        comments: s.comments,
        shares: s.shares,
        dayLabel: dayKey(s.capturedAt).slice(5),
      },
    ]
  }
  const out: TrendDensePoint[] = []
  const sub = TREND_RACE_SUBSTEPS
  for (let i = 0; i < n - 1; i++) {
    const a = snaps[i]!
    const b = snaps[i + 1]!
    for (let s = 0; s < sub; s++) {
      const t = s / sub
      out.push({
        x: i + t,
        likes: lerp(a.likes, b.likes, t),
        collects: lerp(a.collects, b.collects, t),
        comments: lerp(a.comments, b.comments, t),
        shares: lerp(a.shares, b.shares, t),
        dayLabel: dayKey(a.capturedAt).slice(5),
      })
    }
  }
  const last = snaps[n - 1]!
  out.push({
    x: n - 1,
    likes: last.likes,
    collects: last.collects,
    comments: last.comments,
    shares: last.shares,
    dayLabel: dayKey(last.capturedAt).slice(5),
  })
  return out
}

/** 环形图高度跟随左侧账号卡片总高度 */
function syncHealthChartHeight() {
  const cards = healthCardsEl.value
  const chart = healthChartEl.value
  if (!cards || !chart) return
  const h = Math.max(120, Math.ceil(cards.getBoundingClientRect().height))
  chart.style.height = `${h}px`
  healthChart?.resize()
}

type HealthLogTone = 'neutral' | 'step' | 'cyan' | 'gold' | 'ok' | 'warn'
type HealthLogEntry = { id: number; time: string; tag: string; message: string; tone: HealthLogTone }

const healthLogs = ref<HealthLogEntry[]>([])
let healthLogSeq = 0
let healthLogTimer: ReturnType<typeof setTimeout> | null = null

const HEALTH_LOG_SCRIPTS: { tag: string; message: string; tone: HealthLogTone; delayMs: number }[] = [
  { tag: '启动巡检', message: '矩阵健康探针已挂载 · 展示链路（非真实下发）', tone: 'neutral', delayMs: 900 },
  { tag: '步骤 1/4', message: '读取 PC 搜索账号 Cookie 绑定态…', tone: 'step', delayMs: 1400 },
  { tag: '步骤 2/4', message: '探测桌面 CDP 会话在线 / 登录标记…', tone: 'cyan', delayMs: 1600 },
  { tag: '心跳', message: 'Cookie 心跳正常 · 延迟 42ms', tone: 'ok', delayMs: 1200 },
  { tag: '步骤 3/4', message: '汇总可用搜索账号与桌面在线登录占比…', tone: 'step', delayMs: 1500 },
  { tag: '健康', message: '环比健康度刷新完成 · 写入仪表盘环形图', tone: 'ok', delayMs: 1300 },
  { tag: '步骤 4/4', message: '异常队列扫描：暂无告警项', tone: 'cyan', delayMs: 1400 },
  { tag: '攒批等待', message: '下一轮巡检排队中 · 拟人间隔抖动 ±3s', tone: 'gold', delayMs: 2200 },
  { tag: '执行完毕', message: '本轮账号健康快照已落展示层', tone: 'ok', delayMs: 1800 },
  { tag: '待命', message: '控制台空闲 · 持续滚动刷新仅用于看板动效', tone: 'neutral', delayMs: 2500 },
]

function fmtLogClock(d = new Date()) {
  const p = (n: number) => String(n).padStart(2, '0')
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
}

function pushHealthLog(tag: string, message: string, tone: HealthLogTone) {
  healthLogSeq += 1
  healthLogs.value.push({
    id: healthLogSeq,
    time: fmtLogClock(),
    tag,
    message,
    tone,
  })
  if (healthLogs.value.length > 36) {
    healthLogs.value.splice(0, healthLogs.value.length - 36)
  }
  void nextTick(() => {
    const el = healthLogEl.value
    if (el) el.scrollTop = el.scrollHeight
  })
}

function scheduleHealthLogLoop(i = 0) {
  const item = HEALTH_LOG_SCRIPTS[i % HEALTH_LOG_SCRIPTS.length]!
  pushHealthLog(item.tag, item.message, item.tone)
  healthLogTimer = setTimeout(() => scheduleHealthLogLoop(i + 1), item.delayMs)
}

function startHealthLogDemo() {
  if (healthLogTimer) return
  pushHealthLog('待命', '账号健康实时日志已就绪 · 展示动效', 'neutral')
  healthLogTimer = setTimeout(() => scheduleHealthLogLoop(0), 700)
}

function stopHealthLogDemo() {
  if (healthLogTimer) {
    clearTimeout(healthLogTimer)
    healthLogTimer = null
  }
}

const agentLogEl = ref<HTMLElement | null>(null)
const agentLogs = ref<HealthLogEntry[]>([])
let agentLogSeq = 0
let agentLogTimer: ReturnType<typeof setTimeout> | null = null

const AGENT_LOG_SCRIPTS: { tag: string; message: string; tone: HealthLogTone; delayMs: number }[] = [
  { tag: '待命', message: '云端 Agent 舱已就绪 · 展示链路（非真实下发）', tone: 'neutral', delayMs: 1200 },
  { tag: '读配置', message: '加载用途备注与行业方向…', tone: 'step', delayMs: 1400 },
  { tag: 'DeepSeek', message: '根据备注/行业生成搜索关键词…', tone: 'cyan', delayMs: 1600 },
  { tag: '出词', message: '候选词打分完成 · 写入本轮搜索词', tone: 'ok', delayMs: 1300 },
  { tag: '步骤 1/3', message: '选用 PC 搜索账号身份发起最热检索…', tone: 'step', delayMs: 1500 },
  { tag: '步骤 2/3', message: '按 popular 排序拉取候选 Top5…', tone: 'cyan', delayMs: 1600 },
  { tag: '封面', message: '热帖封面落盘 note-media/industry-insight/', tone: 'ok', delayMs: 1400 },
  { tag: '步骤 3/3', message: '汇总互动数据并请求写作建议…', tone: 'step', delayMs: 1500 },
  { tag: '攒批等待', message: '下一轮按配置间隔排队 · 拟人抖动 ±5s', tone: 'gold', delayMs: 2200 },
  { tag: '执行完毕', message: '本轮今日最火快照已写入展示层', tone: 'ok', delayMs: 1800 },
]

function pushAgentLog(tag: string, message: string, tone: HealthLogTone) {
  agentLogSeq += 1
  agentLogs.value.push({
    id: agentLogSeq,
    time: fmtLogClock(),
    tag,
    message,
    tone,
  })
  if (agentLogs.value.length > 36) {
    agentLogs.value.splice(0, agentLogs.value.length - 36)
  }
  void nextTick(() => {
    const el = agentLogEl.value
    if (el) el.scrollTop = el.scrollHeight
  })
}

function scheduleAgentLogLoop(i = 0) {
  const item = AGENT_LOG_SCRIPTS[i % AGENT_LOG_SCRIPTS.length]!
  pushAgentLog(item.tag, item.message, item.tone)
  agentLogTimer = setTimeout(() => scheduleAgentLogLoop(i + 1), item.delayMs)
}

function startAgentLogDemo() {
  if (agentLogTimer) return
  pushAgentLog('待命', '今日最火 Agent 日志已就绪', 'neutral')
  agentLogTimer = setTimeout(() => scheduleAgentLogLoop(0), 800)
}

function stopAgentLogDemo() {
  if (agentLogTimer) {
    clearTimeout(agentLogTimer)
    agentLogTimer = null
  }
}

type DemoLogScript = { tag: string; message: string; tone: HealthLogTone; delayMs: number }

function createDemoLogLoop(scripts: DemoLogScript[], readyMsg: string) {
  const logs = ref<HealthLogEntry[]>([])
  const el = ref<HTMLElement | null>(null)
  let seq = 0
  let timer: ReturnType<typeof setTimeout> | null = null

  function push(tag: string, message: string, tone: HealthLogTone) {
    seq += 1
    logs.value.push({ id: seq, time: fmtLogClock(), tag, message, tone })
    if (logs.value.length > 28) logs.value.splice(0, logs.value.length - 28)
    void nextTick(() => {
      if (el.value) el.value.scrollTop = el.value.scrollHeight
    })
  }

  function loop(i = 0) {
    const item = scripts[i % scripts.length]!
    push(item.tag, item.message, item.tone)
    timer = setTimeout(() => loop(i + 1), item.delayMs)
  }

  function start() {
    if (timer) return
    push('待命', readyMsg, 'neutral')
    timer = setTimeout(() => loop(0), 600 + Math.floor(Math.random() * 400))
  }

  function stop() {
    if (timer) {
      clearTimeout(timer)
      timer = null
    }
  }

  return { logs, el, start, stop }
}

/** 哨兵 / 值守 / 拓客：展示用执行日志（非真实下发） */
const sentinelDemo = createDemoLogLoop(
  [
    { tag: '启动', message: '哨兵 Agent 挂载竞品监控队列 · 展示链路', tone: 'neutral', delayMs: 1100 },
    { tag: '步骤 1/4', message: '扫描到期笔记 · 错开拟人间隔…', tone: 'step', delayMs: 1500 },
    { tag: '步骤 2/4', message: '拉取互动增量 · 写入 competitor_snapshots', tone: 'cyan', delayMs: 1600 },
    { tag: '快照', message: '赞/藏/评/转已落盘 · source=live', tone: 'ok', delayMs: 1300 },
    { tag: '步骤 3/4', message: '商机关键词扫描评论区…', tone: 'step', delayMs: 1500 },
    { tag: '步骤 4/4', message: '更新 next_run_at · 排队下一轮', tone: 'cyan', delayMs: 1400 },
    { tag: '攒批等待', message: '监控空闲窗口 · 抖动 ±10 分钟', tone: 'gold', delayMs: 2200 },
    { tag: '执行完毕', message: '本轮哨兵巡检完成', tone: 'ok', delayMs: 1800 },
  ],
  '哨兵 Agent 日志已就绪 · 笔记监控展示动效',
)
const {
  logs: sentinelLogs,
  el: sentinelLogEl,
  start: startSentinelLog,
  stop: stopSentinelLog,
} = sentinelDemo

const dutyDemo = createDemoLogLoop(
  [
    { tag: '启动', message: '值守 Agent 接管 CDP 会话 · 展示链路', tone: 'neutral', delayMs: 1100 },
    { tag: '步骤 1/5', message: '读取未读角标 / 会话列表…', tone: 'step', delayMs: 1500 },
    { tag: '步骤 2/5', message: '向量检索知识库 · 命中候选回复', tone: 'cyan', delayMs: 1600 },
    { tag: '锁会话', message: '锁定用户句柄，避免重复回复', tone: 'gold', delayMs: 1400 },
    { tag: '步骤 3/5', message: 'DeepSeek 润色客服话术…', tone: 'step', delayMs: 1500 },
    { tag: '发送', message: '已模拟发送私信（展示链路）', tone: 'ok', delayMs: 1300 },
    { tag: '步骤 4/5', message: '回写会话状态 · 清除角标', tone: 'cyan', delayMs: 1400 },
    { tag: '攒批等待', message: '下一轮轮询排队中…', tone: 'gold', delayMs: 2200 },
    { tag: '执行完毕', message: '本轮值守循环完成', tone: 'ok', delayMs: 1800 },
  ],
  '值守 Agent 日志已就绪 · AI 客服展示动效',
)
const { logs: dutyLogs, el: dutyLogEl, start: startDutyLog, stop: stopDutyLog } = dutyDemo

const scoutDemo = createDemoLogLoop(
  [
    { tag: '启动', message: '拓客 Agent 打开获客流水线 · 展示链路', tone: 'neutral', delayMs: 1100 },
    { tag: '步骤 1/4', message: '按关键词搜索笔记 · 排序 popular', tone: 'step', delayMs: 1500 },
    { tag: '步骤 2/4', message: '筛选评论区高意向用户…', tone: 'cyan', delayMs: 1600 },
    { tag: '匹配', message: '命中评论关键词 · 生成触达草稿', tone: 'ok', delayMs: 1400 },
    { tag: '步骤 3/4', message: '拟人间隔后模拟互动/私信…', tone: 'step', delayMs: 1500 },
    { tag: '步骤 4/4', message: '写入获客结果摘要', tone: 'cyan', delayMs: 1400 },
    { tag: '攒批等待', message: '冷却窗口中 · 防风控抖动', tone: 'gold', delayMs: 2200 },
    { tag: '执行完毕', message: '本轮拓客批次完成', tone: 'ok', delayMs: 1800 },
  ],
  '拓客 Agent 日志已就绪 · AI 获客展示动效',
)
const { logs: scoutLogs, el: scoutLogEl, start: startScoutLog, stop: stopScoutLog } = scoutDemo

const pcOk = computed(() => pcAccounts.value.filter((a) => a.loggedIn).length)
const cdpOk = computed(() => cdpRows.value.filter((r) => r.cdpOnline && r.loggedIn).length)
const healthSummary = computed(
  () =>
    `${pcOk.value}/${pcAccounts.value.length} 搜索账号可用 · ${cdpOk.value}/${cdpRows.value.length} 桌面在线登录`,
)

const publishBusy = computed(() => isPublishJobRunning('xhs'))

type HotAgentPhase = 'need_config' | 'need_account' | 'running' | 'error' | 'standby'

const ADVICE_PLACEHOLDER =
  '暂无运营建议。配置备注/行业并刷新后，将在此以打字机效果循环展示笔记写作建议。'

const ADVICE_HOLD_MS = 8000

const adviceTyped = ref('')
const adviceTyping = ref(false)
const adviceTypeEl = ref<HTMLElement | null>(null)
/** 打完后的停留阶段 */
const adviceHolding = ref(false)
/** 鼠标在区域内：停留计时暂停，方便复制 */
const adviceHover = ref(false)
let adviceTypeTimer: ReturnType<typeof setTimeout> | null = null
let adviceLoopTimer: ReturnType<typeof setTimeout> | null = null
/** 因悬停打断停留后，离开再重新计 8 秒 */
let adviceHoldPaused = false

function scrollAdviceTypeToEnd() {
  void nextTick(() => {
    const el = adviceTypeEl.value
    if (el) el.scrollTop = el.scrollHeight
  })
}

const adviceScript = computed(() => {
  const a = insightAnalysis.value
  if (!a) return ''
  const parts: string[] = []
  if (a.summary?.trim()) parts.push(a.summary.trim())
  if (a.hooks?.length) {
    const hooks = a.hooks.map((h) => String(h).trim()).filter(Boolean)
    if (hooks.length) parts.push(`钩子：${hooks.join('；')}`)
  }
  if (a.structure?.trim()) parts.push(`结构：${a.structure.trim()}`)
  if (a.suggestions?.length) {
    parts.push(
      a.suggestions
        .map((s, i) => `${i + 1}. ${String(s).trim()}`)
        .filter((s) => s.length > 2)
        .join('\n'),
    )
  }
  return parts.filter(Boolean).join('\n\n')
})

function clearAdviceTypeTimers() {
  if (adviceTypeTimer) {
    clearTimeout(adviceTypeTimer)
    adviceTypeTimer = null
  }
  if (adviceLoopTimer) {
    clearTimeout(adviceLoopTimer)
    adviceLoopTimer = null
  }
}

function clearAdviceHoldTimer() {
  if (adviceLoopTimer) {
    clearTimeout(adviceLoopTimer)
    adviceLoopTimer = null
  }
}

function finishAdviceHoldAndRestart() {
  adviceHolding.value = false
  adviceHoldPaused = false
  adviceTyped.value = ''
  adviceLoopTimer = setTimeout(() => startAdviceTypewriter(), 900)
}

/** 打完后停留 8 秒；悬停则不进入清空/重播 */
function startAdviceHoldTimer() {
  clearAdviceHoldTimer()
  adviceHolding.value = true
  adviceHoldPaused = false
  if (adviceHover.value) {
    adviceHoldPaused = true
    return
  }
  adviceLoopTimer = setTimeout(() => {
    if (adviceHover.value) {
      adviceHoldPaused = true
      return
    }
    finishAdviceHoldAndRestart()
  }, ADVICE_HOLD_MS)
}

function onAdviceAreaEnter() {
  adviceHover.value = true
  if (adviceHolding.value) {
    clearAdviceHoldTimer()
    adviceHoldPaused = true
  }
}

function onAdviceAreaLeave() {
  adviceHover.value = false
  if (adviceHolding.value && adviceHoldPaused) {
    startAdviceHoldTimer()
  }
}

function startAdviceTypewriter() {
  clearAdviceTypeTimers()
  adviceHolding.value = false
  adviceHoldPaused = false
  const full = adviceScript.value
  if (!full) {
    adviceTyped.value = ''
    adviceTyping.value = false
    return
  }
  adviceTyping.value = true
  adviceTyped.value = ''
  let i = 0
  const tick = () => {
    // 文案已切换则中止本轮
    if (full !== adviceScript.value) return
    i += 1
    adviceTyped.value = full.slice(0, i)
    scrollAdviceTypeToEnd()
    if (i < full.length) {
      const ch = full[i - 1] || ''
      const delay =
        ch === '\n'
          ? 200
          : ch === '，' || ch === '。' || ch === '；' || ch === '！' || ch === '？' || ch === ':'
            ? 160
            : 58
      adviceTypeTimer = setTimeout(tick, delay)
    } else {
      adviceTyping.value = false
      scrollAdviceTypeToEnd()
      startAdviceHoldTimer()
    }
  }
  adviceTypeTimer = setTimeout(tick, 450)
}

watch(
  adviceScript,
  (script, prev) => {
    if (script === prev && adviceTyping.value) return
    startAdviceTypewriter()
  },
  { immediate: true },
)

async function copyAdviceText() {
  const text = (adviceScript.value || adviceTyped.value || '').trim()
  if (!text) {
    ElMessage.warning('暂无建议可复制')
    return
  }
  try {
    await navigator.clipboard.writeText(text)
    ElMessage.success('已复制运营建议')
  } catch {
    ElMessage.error('复制失败，请手动选中文本')
  }
}

const hasInsightSeed = computed(() => {
  const industry = (insightConfig.value?.industryKeyword || '').trim()
  const remark = (insightConfig.value?.remark || '').trim()
  return Boolean(industry || remark)
})

const hotAgentPhase = computed<HotAgentPhase>(() => {
  if (!hasInsightSeed.value) return 'need_config'
  if (!insightAccount.value) return 'need_account'
  if (insightRunning.value) return 'running'
  if (insightLastError.value) return 'error'
  return 'standby'
})

const hotAgentTitle = computed(() => {
  if (hotAgentPhase.value === 'need_config') return '等待配置'
  if (hotAgentPhase.value === 'need_account') return '等待搜索账号'
  if (hotAgentPhase.value === 'error') return '本轮异常'
  if (hotAgentPhase.value === 'running') {
    if (insightPhase.value === 'expand') return '生成搜索词'
    if (insightPhase.value === 'analyze') return 'AI 写作分析'
    return '云端采集中'
  }
  return '今日最火 Agent · 云端值守中'
})

const hotAgentDesc = computed(() => {
  switch (hotAgentPhase.value) {
    case 'need_config':
      return '配置备注/行业后，DeepSeek 会先产出搜索词再拉最热帖'
    case 'need_account':
      return '请绑定已登录的 PC 搜索账号作为采集身份'
    case 'running':
      if (insightPhase.value === 'expand') return 'DeepSeek 根据备注与行业生成搜索关键词…'
      if (insightPhase.value === 'search') {
        return insightSearchQuery.value
          ? `正在用「${insightSearchQuery.value}」搜索最热帖…`
          : '正在搜索最热帖…'
      }
      if (insightPhase.value === 'analyze') return '热帖已到手，正在生成笔记写作建议…'
      return 'Agent 正在云端运行…'
    case 'error':
      return insightLastError.value || '已跳过本轮，稍后自动重试'
    default:
      if (insightSearchQuery.value) {
        return `上轮搜索词「${insightSearchQuery.value}」· 约 ${insightIntervalMin.value} 分钟后再跑`
      }
      return insightLastRun.value?.createdAt
        ? `上轮完成 · ${insightLastRun.value.createdAt} · 约 ${insightIntervalMin.value} 分钟后再跑`
        : `已就绪 · 约每 ${insightIntervalMin.value} 分钟自动运行`
  }
})

function fmt(n: number) {
  if (n >= 10000) return `${(n / 10000).toFixed(1)}万`
  return String(n)
}

function hotCoverSrc(p: HotPost): string {
  const local = String(p.coverLocalPath || '').trim()
  const remote = String(p.coverUrl || '').trim()
  return mediaDisplaySrc(local || remote)
}

function shortId(id: string, keep = 8) {
  const s = String(id || '').trim()
  if (!s || s.length <= keep + 2) return s || '—'
  return `${s.slice(0, keep)}…`
}

function dayKey(capturedAt: string) {
  const t = String(capturedAt || '')
  return t.length >= 10 ? t.slice(0, 10) : t
}

/** 同日取最后一条快照 */
function aggregateDaily(snaps: SnapshotRow[]): SnapshotRow[] {
  const map = new Map<string, SnapshotRow>()
  for (const s of snaps) {
    map.set(dayKey(s.capturedAt), s)
  }
  return [...map.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([, v]) => v)
}

function dailyGrowth(snaps: SnapshotRow[]): number {
  const daily = aggregateDaily(snaps)
  if (daily.length < 2) return 0
  const first = daily[0]!
  const last = daily[daily.length - 1]!
  const days = Math.max(1, daily.length - 1)
  return (last.engagement - first.engagement) / days
}

/** 多笔记按日合计赞/藏/评/转（同日各笔记末日快照相加） */
function aggregateTotalDaily(byId: Record<string, SnapshotRow[]>): SnapshotRow[] {
  const dayMap = new Map<
    string,
    { likes: number; collects: number; comments: number; shares: number; engagement: number }
  >()
  for (const snaps of Object.values(byId)) {
    for (const s of aggregateDaily(snaps)) {
      const k = dayKey(s.capturedAt)
      const cur = dayMap.get(k) || {
        likes: 0,
        collects: 0,
        comments: 0,
        shares: 0,
        engagement: 0,
      }
      cur.likes += s.likes
      cur.collects += s.collects
      cur.comments += s.comments
      cur.shares += s.shares
      cur.engagement += s.engagement
      dayMap.set(k, cur)
    }
  }
  return [...dayMap.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([day, v], i) => ({
      id: i,
      competitorId: 'total',
      likes: v.likes,
      collects: v.collects,
      comments: v.comments,
      shares: v.shares,
      engagement: v.engagement,
      capturedAt: day,
    }))
}

function parsePcAccounts(env: { data?: unknown }): PcAccount[] {
  const data = env.data
  const list: unknown[] = Array.isArray(data)
    ? data
    : data && typeof data === 'object' && Array.isArray((data as { accounts?: unknown[] }).accounts)
      ? (data as { accounts: unknown[] }).accounts
      : data && typeof data === 'object' && Array.isArray((data as { list?: unknown[] }).list)
        ? (data as { list: unknown[] }).list
        : []
  return list
    .map((x) => {
      if (!x || typeof x !== 'object') return null
      const r = x as Record<string, unknown>
      const user =
        r.user && typeof r.user === 'object' ? (r.user as Record<string, unknown>) : {}
      const acct = String(r.account || r.name || 'default').trim()
      if (!acct) return null
      return {
        account: acct,
        nickname: String(user.nickname || user.name || r.nickname || acct),
        loggedIn: r.logged_in === true || r.loggedIn === true || Boolean(user.id || user.uid),
        avatarUrl: String(user.avatar || user.avatarUrl || r.avatarUrl || ''),
        externalUserId: String(user.id || user.uid || ''),
      } satisfies PcAccount
    })
    .filter(Boolean) as PcAccount[]
}

async function loadHealth() {
  healthBusy.value = true
  try {
    const env = await authList(PLATFORM)
    let rows = parsePcAccounts(env)
    if (!rows.length) {
      try {
        const st = await authStatus(PLATFORM, 'default')
        const data = (st.data && typeof st.data === 'object' ? st.data : {}) as Record<
          string,
          unknown
        >
        const user =
          data.user && typeof data.user === 'object'
            ? (data.user as Record<string, unknown>)
            : {}
        const loggedIn = Boolean(data.logged_in || data.loggedIn || user.id)
        if (loggedIn || Object.keys(data).length) {
          rows = [
            {
              account: 'default',
              nickname: String(user.nickname || user.name || 'default'),
              loggedIn,
              avatarUrl: String(user.avatar || user.avatarUrl || ''),
              externalUserId: String(user.id || user.uid || ''),
            },
          ]
        }
      } catch {
        /* optional */
      }
    }
    pcAccounts.value = rows

    const profiles = await listBrowserProfiles()
    let statuses: BrowserStatus[] = []
    try {
      const sum = await browserCdpStatusAll()
      statuses = sum.profiles || []
    } catch {
      statuses = []
    }
    cdpRows.value = profiles.map((p: BrowserProfile) => {
      const st = statuses.find((s) => s.profileId === p.id)
      const sess = parseProfileSession(st?.sessionJson || p.sessionJson).xhs
      return {
        id: p.id,
        name: p.name,
        cdpOnline: Boolean(st?.cdpOnline),
        loggedIn: Boolean(sess?.loggedIn),
        nickname: String(sess?.nickname || ''),
        avatarUrl: String(sess?.avatarUrl || ''),
        followers: String(sess?.followers || '—'),
        following: String(sess?.following || '—'),
        likesCollect: String(sess?.likesCollect || '—'),
        redId: String(sess?.redId || ''),
      }
    })
    await nextTick()
    renderHealthChart()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    healthBusy.value = false
  }
}

function renderHealthChart() {
  if (!healthChartEl.value) return
  if (!healthChart) healthChart = echarts.init(healthChartEl.value)
  const healthy =
    pcAccounts.value.filter((a) => a.loggedIn).length +
    cdpRows.value.filter((r) => r.cdpOnline && r.loggedIn).length
  const bad =
    pcAccounts.value.filter((a) => !a.loggedIn).length +
    cdpRows.value.filter((r) => !(r.cdpOnline && r.loggedIn)).length
  const total = healthy + bad
  const pct = total > 0 ? Math.round((healthy / total) * 100) : 0
  healthChart.setOption(
    {
      tooltip: { trigger: 'item' },
      series: [
        {
          type: 'pie',
          radius: ['55%', '78%'],
          center: ['50%', '50%'],
          avoidLabelOverlap: false,
          label: { show: false },
          labelLine: { show: false },
          data: [
            { name: '健康', value: healthy, itemStyle: { color: '#67c23a' } },
            { name: '异常', value: bad, itemStyle: { color: '#909399' } },
          ].filter((d) => d.value > 0),
        },
      ],
      graphic: [
        {
          type: 'text',
          left: 'center',
          top: '42%',
          style: {
            text: total ? `${pct}%` : '—',
            fill: '#303133',
            fontSize: 16,
            fontWeight: 600,
            textAlign: 'center',
          },
        },
        {
          type: 'text',
          left: 'center',
          top: '58%',
          style: {
            text: '健康',
            fill: '#909399',
            fontSize: 11,
            textAlign: 'center',
          },
        },
      ],
    },
    true,
  )
  void nextTick(() => syncHealthChartHeight())
}

async function loadConfigDrafts() {
  await insightLoad()
  remarkDraft.value = insightConfig.value?.remark || ''
  keywordDraft.value = insightConfig.value?.industryKeyword || ''
  intervalDraft.value = insightIntervalMin.value
}

async function saveConfig() {
  const keyword = keywordDraft.value.trim()
  const remark = remarkDraft.value.trim()
  const interval = Math.min(1440, Math.max(15, Math.round(Number(intervalDraft.value) || 30)))
  if (!keyword && !remark) {
    ElMessage.warning('请至少填写用途备注或行业关键词')
    return
  }
  configSaving.value = true
  try {
    const prevKw = insightConfig.value?.industryKeyword || ''
    const prevRemark = insightConfig.value?.remark || ''
    const cfg = await dashboardInsightConfigUpsert({
      platform: PLATFORM,
      remark,
      industryKeyword: keyword,
      intervalMinutes: interval,
    })
    setConfigLocal(cfg)
    configDialogOpen.value = false
    ElMessage.success('已保存')
    if (prevKw !== keyword || prevRemark !== remark) {
      try {
        await ElMessageBox.confirm(
          '配置已变更，DeepSeek 将根据备注/行业生成搜索词再拉热帖，是否立即刷新？',
          '提示',
          {
            type: 'info',
            confirmButtonText: '立即刷新',
            cancelButtonText: '稍后',
          },
        )
        await insightRefresh()
      } catch {
        /* cancel */
      }
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    configSaving.value = false
  }
}

async function loadCompetitorsBlock() {
  try {
    competitors.value = await competitorList()
    const users = await watchedUserList()
    watchedCount.value = users.length
    unreadEvents.value = users.reduce((s, u) => s + (u.unreadEvents || 0), 0)

    // 进仪表盘自动补齐近 30 天缺失快照（线性爬升，source=backfill）
    try {
      await competitorSnapshotsBackfill({ days: TREND_DAYS })
    } catch {
      /* 补齐失败不阻断图表 */
    }

    const scored: typeof growthRows.value = []
    const cache: Record<string, SnapshotRow[]> = {}
    await Promise.all(
      competitors.value.map(async (row) => {
        try {
          const snaps = await competitorSnapshots(row.id, TREND_DAYS)
          cache[row.id] = snaps
          const delta = dailyGrowth(snaps)
          scored.push({
            id: row.id,
            title: row.title || row.noteUrl,
            authorName: row.authorName,
            dailyDelta: Math.round(delta * 10) / 10,
            engagement: row.likes + row.collects + row.comments + row.shares,
          })
        } catch {
          /* skip */
        }
      }),
    )
    snapsByCompetitor.value = cache
    growthRows.value = scored.sort((a, b) => b.dailyDelta - a.dailyDelta).slice(0, 5)
    // 默认展示全部合计 Total
    if (chartMode.value === 'note' && selectedNoteId.value) {
      await loadChart()
    } else {
      await showTotalChart()
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function showTotalChart() {
  chartMode.value = 'total'
  selectedNoteId.value = ''
  chartBusy.value = true
  try {
    let cache = snapsByCompetitor.value
    if (!Object.keys(cache).length && competitors.value.length) {
      const next: Record<string, SnapshotRow[]> = {}
      await Promise.all(
        competitors.value.map(async (row) => {
          try {
            next[row.id] = await competitorSnapshots(row.id, TREND_DAYS)
          } catch {
            /* skip */
          }
        }),
      )
      snapsByCompetitor.value = next
      cache = next
    }
    chartSnaps.value = aggregateTotalDaily(cache)
    await nextTick()
    renderTrendChart()
  } finally {
    chartBusy.value = false
  }
}

async function loadChart() {
  if (!selectedNoteId.value) {
    await showTotalChart()
    return
  }
  chartMode.value = 'note'
  chartBusy.value = true
  try {
    let snaps = snapsByCompetitor.value[selectedNoteId.value]
    if (!snaps) {
      snaps = await competitorSnapshots(selectedNoteId.value, TREND_DAYS)
      snapsByCompetitor.value = {
        ...snapsByCompetitor.value,
        [selectedNoteId.value]: snaps,
      }
    }
    chartSnaps.value = aggregateDaily(snaps)
    await nextTick()
    renderTrendChart()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    chartBusy.value = false
  }
}

/** progress ∈ [0,1]：按密采样路径逐步揭示（日间 24 段插值） */
function buildTrendRaceOption(progress: number): echarts.EChartsOption {
  const snaps = chartSnaps.value
  const n = snaps.length
  const dense = buildTrendDensePath(snaps)
  const p = Math.min(1, Math.max(0, progress))
  const visible = dense.length
    ? dense.slice(0, Math.max(1, Math.ceil(p * (dense.length - 1)) + 1))
    : []
  const allVals = snaps.flatMap((s) => [s.likes, s.collects, s.comments, s.shares])
  const yMax = allVals.length ? Math.max(...allVals) * 1.08 : undefined
  const done = p >= 1
  // 已揭示到的「整日」下标（插值段不进 tooltip）
  const revealedDay = Math.min(n - 1, Math.max(0, Math.floor(p * Math.max(0, n - 1) + 1e-9)))

  return {
    animation: false,
    tooltip: {
      trigger: 'axis',
      // 吸附到整日刻度，避免停在插值点上
      snap: true,
      formatter: (params: unknown) => {
        const arr = Array.isArray(params) ? params : []
        if (!arr.length || n <= 0) return ''
        const first = arr[0] as { data?: number[]; axisValue?: number | string }
        const x = Number(first.data?.[0] ?? first.axisValue ?? 0)
        const i = Math.min(revealedDay, Math.max(0, Math.round(x)))
        const snap = snaps[i]
        if (!snap) return ''
        const head = dayKey(snap.capturedAt).slice(5)
        const metrics: { name: string; value: number; color: string }[] = [
          { name: '赞', value: snap.likes, color: '#5470c6' },
          { name: '藏', value: snap.collects, color: '#91cc75' },
          { name: '评', value: snap.comments, color: '#73767a' },
          { name: '转', value: snap.shares, color: '#fc8452' },
        ]
        const lines = metrics.map(
          (m) =>
            `<span style="display:inline-block;margin-right:6px;border-radius:50%;width:8px;height:8px;background:${m.color}"></span>${m.name}: ${Math.round(m.value)}`,
        )
        return [head, ...lines].join('<br/>')
      },
    },
    legend: { data: ['赞', '藏', '评', '转'], top: 0, left: 'center' },
    grid: { left: 48, right: 20, top: 40, bottom: n > 8 ? 48 : 36 },
    xAxis: {
      type: 'value',
      min: 0,
      max: Math.max(0, n - 1),
      interval: 1,
      splitLine: { show: false },
      axisLabel: {
        hideOverlap: true,
        rotate: n > 8 ? 30 : 0,
        fontSize: 11,
        formatter: (v: number) => {
          if (n <= 0) return ''
          const i = Math.round(v)
          if (Math.abs(v - i) > 0.05) return ''
          if (i < 0 || i >= n) return ''
          return dayKey(snaps[i]!.capturedAt).slice(5)
        },
      },
    },
    yAxis: {
      type: 'value',
      scale: true,
      max: yMax && Number.isFinite(yMax) ? Math.ceil(yMax) : undefined,
    },
    series: [
      {
        name: '赞',
        type: 'line',
        smooth: 0.2,
        showSymbol: done && n <= 20,
        symbolSize: 5,
        data: visible.map((d) => [d.x, d.likes]),
      },
      {
        name: '藏',
        type: 'line',
        smooth: 0.2,
        showSymbol: done && n <= 20,
        symbolSize: 5,
        data: visible.map((d) => [d.x, d.collects]),
      },
      {
        name: '评',
        type: 'line',
        smooth: 0.2,
        showSymbol: done && n <= 20,
        symbolSize: 5,
        data: visible.map((d) => [d.x, d.comments]),
      },
      {
        name: '转',
        type: 'line',
        smooth: 0.2,
        showSymbol: done && n <= 20,
        symbolSize: 5,
        data: visible.map((d) => [d.x, d.shares]),
      },
    ],
  }
}

/** line-race + rAF 插值：平滑爬完 → 停 10s → 再开 */
function replayTrendClimb() {
  if (!trendChartEl.value) return
  if (!trendChart) trendChart = echarts.init(trendChartEl.value)
  stopTrendClimbLoop()
  const snaps = chartSnaps.value
  if (!snaps.length) {
    trendChart.clear()
    trendChart.setOption(buildTrendRaceOption(0), true)
    trendChart.resize()
    return
  }

  trendChart.clear()
  trendChart.setOption(buildTrendRaceOption(0), true)
  trendChart.resize()

  const started = performance.now()
  const tick = (now: number) => {
    const raw = Math.min(1, (now - started) / TREND_RACE_MS)
    const eased = easeInOutSine(raw)
    trendChart?.setOption(buildTrendRaceOption(eased), { lazyUpdate: true })
    if (raw < 1) {
      trendRaceRaf = requestAnimationFrame(tick)
      return
    }
    trendRaceRaf = 0
    trendChart?.setOption(buildTrendRaceOption(1))
    trendHoldTimer = setTimeout(() => {
      trendHoldTimer = null
      replayTrendClimb()
    }, TREND_HOLD_MS)
  }
  trendRaceRaf = requestAnimationFrame(tick)
}

function renderTrendChart() {
  stopTrendClimbLoop()
  replayTrendClimb()
}

function pickGrowth(id: string) {
  selectedNoteId.value = id
  chartMode.value = 'note'
  void loadChart()
}

function resetGrowthChart() {
  void showTotalChart()
}

function onResize() {
  syncHealthChartHeight()
  trendChart?.resize()
}

onMounted(async () => {
  window.addEventListener('resize', onResize)
  startHealthLogDemo()
  startAgentLogDemo()
  startSentinelLog()
  startDutyLog()
  startScoutLog()
  await Promise.all([loadHealth(), loadConfigDrafts(), loadCompetitorsBlock()])
  void refreshImageJobs()
  await nextTick()
  if (healthCardsEl.value && typeof ResizeObserver !== 'undefined') {
    healthCardsRo = new ResizeObserver(() => syncHealthChartHeight())
    healthCardsRo.observe(healthCardsEl.value)
  }
  syncHealthChartHeight()
})

onUnmounted(() => {
  window.removeEventListener('resize', onResize)
  stopHealthLogDemo()
  stopAgentLogDemo()
  stopSentinelLog()
  stopDutyLog()
  stopScoutLog()
  clearAdviceTypeTimers()
  stopTrendClimbLoop()
  healthCardsRo?.disconnect()
  healthCardsRo = null
  healthChart?.dispose()
  trendChart?.dispose()
  healthChart = null
  trendChart = null
})
</script>

<template>
  <div class="xhs-dash">
    <div class="dash-row dash-row--health-hot">
      <!-- 矩阵账号健康 -->
      <section class="mf-ops__panel dash-card">
        <div class="mf-ops__panel-head">
          <div>
            <strong>矩阵账号健康</strong>
            <span class="dash-sub">{{ healthSummary }}</span>
          </div>
          <div class="dash-actions">
            <el-button size="small" :loading="healthBusy" @click="loadHealth">刷新状态</el-button>
            <el-button size="small" @click="router.push('/xhs/accounts')">去账号矩阵</el-button>
          </div>
        </div>
        <div class="mf-ops__panel-body dash-health" v-loading="healthBusy">
          <div class="dash-health__top">
            <div ref="healthCardsEl" class="dash-health__main">
              <div v-if="!pcAccounts.length && !cdpRows.length" class="dash-empty">
                暂无账号，请先去账号矩阵绑定搜索账号或桌面档案。
                <el-button size="small" type="primary" link @click="router.push('/xhs/accounts')">
                  去绑定
                </el-button>
              </div>
              <div v-else class="mf-ops__grid dash-acc-grid">
                <article v-for="a in pcAccounts" :key="'pc-' + a.account" class="mf-ops__card">
                  <div class="mf-ops__card-head">
                    <div class="mf-ops__card-avatar">
                      <img
                        v-if="a.avatarUrl"
                        :src="a.avatarUrl"
                        alt=""
                        referrerpolicy="no-referrer"
                      />
                      <span v-else>{{ (a.nickname || '?').slice(0, 1) }}</span>
                    </div>
                    <div class="mf-ops__card-meta">
                      <strong>{{ a.nickname || '未命名账号' }}</strong>
                      <em>PC 搜索 · {{ a.account }}</em>
                    </div>
                    <el-tag size="small" :type="a.loggedIn ? 'success' : 'info'">
                      {{ a.loggedIn ? '正常' : '未知' }}
                    </el-tag>
                  </div>
                  <div class="mf-ops__card-stats">
                    <div><span>Cookie</span><b>{{ a.loggedIn ? '已绑定' : '无' }}</b></div>
                    <div>
                      <span>UID</span>
                      <b :title="a.externalUserId || ''">{{ shortId(a.externalUserId, 6) }}</b>
                    </div>
                  </div>
                </article>
                <article v-for="p in cdpRows" :key="'cdp-' + p.id" class="mf-ops__card">
                  <div class="mf-ops__card-head">
                    <div class="mf-ops__card-avatar">
                      <img
                        v-if="p.avatarUrl"
                        :src="p.avatarUrl"
                        alt=""
                        referrerpolicy="no-referrer"
                      />
                      <span v-else>{{ (p.nickname || p.name || '?').slice(0, 1) }}</span>
                    </div>
                    <div class="mf-ops__card-meta">
                      <strong>{{ p.nickname || p.name }}</strong>
                      <em>
                        桌面 CDP
                        <template v-if="p.redId"> · 号 {{ p.redId }}</template>
                      </em>
                    </div>
                    <el-tag
                      size="small"
                      :type="p.cdpOnline && p.loggedIn ? 'success' : p.cdpOnline ? 'warning' : 'info'"
                    >
                      {{
                        p.cdpOnline && p.loggedIn
                          ? '在线·已登录'
                          : p.cdpOnline
                            ? '在线·未登录'
                            : '离线'
                      }}
                    </el-tag>
                  </div>
                  <div class="mf-ops__card-stats">
                    <div><span>粉丝</span><b>{{ p.followers }}</b></div>
                    <div><span>关注</span><b>{{ p.following }}</b></div>
                    <div><span>获赞</span><b>{{ p.likesCollect }}</b></div>
                  </div>
                </article>
              </div>
            </div>
            <div ref="healthChartEl" class="dash-health__chart" />
          </div>
          <div class="dash-health-log" aria-hidden="true">
            <div class="dash-health-log__head">
              <span class="dash-health-log__dot" />
              <span class="dash-health-log__title">账号健康 · 实时巡检日志</span>
            </div>
            <div ref="healthLogEl" class="dash-health-log__body">
              <div
                v-for="entry in healthLogs"
                :key="entry.id"
                class="dash-hlog"
                :class="`tone-${entry.tone}`"
              >
                <span class="dash-hlog__time">[{{ entry.time }}]</span>
                <span class="dash-hlog__tag">[{{ entry.tag }}]</span>
                <span class="dash-hlog__msg">{{ entry.message }}</span>
              </div>
            </div>
          </div>
        </div>
      </section>

      <!-- 今日最火 -->
      <section class="mf-ops__panel dash-card">
        <div class="mf-ops__panel-head">
          <div>
            <strong>今日最火 Top5</strong>
            <span class="dash-sub">
              搜索账号：{{ insightNickname || insightAccount || '未绑定' }}
              <template v-if="insightSearchQuery"> · 词「{{ insightSearchQuery }}」</template>
              <template v-else-if="insightConfig?.industryKeyword">
                · 行业 {{ insightConfig.industryKeyword }}
              </template>
            </span>
          </div>
          <div class="dash-actions">
            <el-button
              size="small"
              :icon="Setting"
              circle
              title="配置用途备注与行业关键词"
              @click="openConfigDialog"
            />
            <el-button size="small" :loading="insightRunning" @click="insightRefresh()">
              立即刷新
            </el-button>
          </div>
        </div>
        <div class="mf-ops__panel-body dash-hot-body">
          <div class="dash-hot-split">
            <!-- 左：热帖 -->
            <div class="dash-hot-left">
              <div class="dash-hot-left__posts">
                <div v-if="!hasInsightSeed" class="dash-hot-left__empty">
                  点击右上角
                  <el-icon class="dash-inline-icon"><Setting /></el-icon>
                  配置备注/行业后，DeepSeek 会生成搜索词再采集
                </div>
                <div v-else-if="!insightAccount" class="dash-hot-left__empty">
                  请先
                  <el-button size="small" type="primary" link @click="router.push('/xhs/accounts')">
                    绑定搜索账号
                  </el-button>
                </div>
                <div v-else-if="!insightPosts.length" class="dash-hot-left__empty">
                  {{ insightRunning ? '正在拉取热帖…' : '暂无热帖，点「立即刷新」或等待定时任务' }}
                </div>
                <div v-else class="dash-hot-grid">
                  <a
                    v-for="(p, i) in insightPosts"
                    :key="p.id || i"
                    class="dash-hot"
                    :href="p.url || '#'"
                    target="_blank"
                    rel="noreferrer"
                    @click="(e) => !p.url && e.preventDefault()"
                  >
                    <img
                      v-if="p.coverLocalPath || p.coverUrl"
                      :src="hotCoverSrc(p)"
                      alt=""
                      referrerpolicy="no-referrer"
                    />
                    <div v-else class="dash-hot__ph" />
                    <div class="dash-hot__body">
                      <b>{{ i + 1 }}. {{ p.title || '无标题' }}</b>
                      <span>{{ p.authorName }}</span>
                      <span>
                        赞 {{ fmt(p.likes || 0) }} · 藏 {{ fmt(p.collects || 0) }} · 评
                        {{ fmt(p.comments || 0) }}
                      </span>
                    </div>
                  </a>
                </div>
              </div>
              <div
                class="dash-advice-type"
                :class="{ 'is-hold-paused': adviceHolding && adviceHover }"
                @mouseenter="onAdviceAreaEnter"
                @mouseleave="onAdviceAreaLeave"
              >
                <div class="dash-advice-type__label">
                  <span>运营建议 · 笔记怎么写</span>
                  <div class="dash-advice-type__actions">
                    <button
                      type="button"
                      class="dash-advice-type__icon-btn"
                      title="复制全文"
                      :disabled="!adviceScript"
                      @click="copyAdviceText()"
                    >
                      <el-icon :size="14"><CopyDocument /></el-icon>
                    </button>
                    <button
                      type="button"
                      class="dash-advice-type__icon-btn"
                      title="重新播放打字机"
                      :disabled="!adviceScript"
                      @click="startAdviceTypewriter()"
                    >
                      <el-icon :size="14"><Refresh /></el-icon>
                    </button>
                  </div>
                </div>
                <div
                  ref="adviceTypeEl"
                  class="dash-advice-type__body"
                  :class="{ 'is-placeholder': !adviceTyped && !adviceTyping }"
                >
                  <template v-if="adviceTyped">{{ adviceTyped }}</template>
                  <template v-else-if="adviceTyping"> </template>
                  <template v-else>{{ ADVICE_PLACEHOLDER }}</template>
                  <span
                    v-if="adviceScript"
                    class="dash-advice-type__caret"
                    :class="{ blink: !adviceTyping && !!adviceTyped }"
                  />
                </div>
              </div>
            </div>

            <!-- 右：云端 Agent = 上 tip + 下日志 -->
            <div class="dash-agent" :class="`is-${hotAgentPhase}`">
              <div class="dash-agent__tip">
                <div class="dash-agent__sky">
                  <span class="dash-agent__cloud c1" />
                  <span class="dash-agent__cloud c2" />
                  <span class="dash-agent__cloud c3" />
                  <span v-for="n in 6" :key="n" class="dash-agent__dot" :style="{ '--i': n }" />
                </div>
                <div class="dash-agent__core">
                  <div class="dash-agent__ring" />
                  <div class="dash-agent__ring dash-agent__ring--outer" />
                  <div class="dash-agent__face" aria-hidden="true">
                    <span class="dash-agent__eye" />
                    <span class="dash-agent__eye" />
                  </div>
                </div>
                <div class="dash-agent__copy">
                  <b>{{ hotAgentTitle }}</b>
                  <span>{{ hotAgentDesc }}</span>
                </div>
              </div>
              <div class="dash-agent-log" aria-hidden="true">
                <div class="dash-agent-log__head">
                  <span class="dash-agent-log__dot" />
                  <span class="dash-agent-log__title">今日最火 · Agent 执行日志</span>
                </div>
                <div ref="agentLogEl" class="dash-agent-log__body">
                  <div
                    v-for="entry in agentLogs"
                    :key="entry.id"
                    class="dash-hlog"
                    :class="`tone-${entry.tone}`"
                  >
                    <span class="dash-hlog__time">[{{ entry.time }}]</span>
                    <span class="dash-hlog__tag">[{{ entry.tag }}]</span>
                    <span class="dash-hlog__msg">{{ entry.message }}</span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>
    </div>

    <!-- 增速 + 折线（紧挨顶部健康/最火区） -->
    <section class="mf-ops__panel dash-card">
      <div class="mf-ops__panel-head">
        <strong>监控笔记增速 Top5</strong>
        <el-button size="small" @click="loadCompetitorsBlock">刷新</el-button>
      </div>
      <div class="mf-ops__panel-body dash-growth">
        <div class="dash-growth__list">
          <div v-if="!growthRows.length" class="dash-empty">暂无监控笔记或快照不足。</div>
          <button
            v-for="(g, i) in growthRows"
            :key="g.id"
            type="button"
            class="dash-growth__item"
            :class="{ active: selectedNoteId === g.id }"
            @click="pickGrowth(g.id)"
          >
            <b>{{ i + 1 }}. {{ g.title }}</b>
            <span>{{ g.authorName }} · 日均互动 +{{ g.dailyDelta }}</span>
          </button>
        </div>
        <div class="dash-growth__chart-wrap" v-loading="chartBusy">
          <div class="dash-chart-toolbar">
            <span class="dash-sub" style="margin-left: 0">
              {{
                chartMode === 'total'
                  ? '近 30 天 · 按日聚合 · Total（全部监控合计）'
                  : `近 30 天 · 按日聚合 · ${
                      competitors.find((c) => c.id === selectedNoteId)?.title || '已选笔记'
                    }`
              }}
            </span>
            <el-button size="small" :disabled="chartMode === 'total'" @click="resetGrowthChart">
              重置
            </el-button>
          </div>
          <div ref="trendChartEl" class="dash-trend-chart" />
          <p v-if="!chartSnaps.length" class="dash-empty">
            暂无快照数据；添加监控笔记并刷新后可查看 Total 趋势。
          </p>
        </div>
      </div>
    </section>

    <!-- 后台运行 -->
    <section class="mf-ops__panel dash-card">
      <div class="mf-ops__panel-head">
        <strong>后台运行状态</strong>
      </div>
      <div class="mf-ops__panel-body dash-runners">
        <div class="dash-agent-card is-on">
          <div class="dash-run">
            <span class="dash-dot is-on" />
            <div class="dash-run__body">
              <b>哨兵 Agent · 笔记监控</b>
              <span>云端值守中</span>
            </div>
            <el-button size="small" text @click="router.push('/xhs/competitors')">去竞品</el-button>
          </div>
          <div class="dash-run-log" aria-hidden="true">
            <div class="dash-run-log__head">
              <span class="dash-run-log__dot" />
              <span class="dash-run-log__title">哨兵 Agent · 执行日志</span>
            </div>
            <div ref="sentinelLogEl" class="dash-run-log__body">
              <div
                v-for="entry in sentinelLogs"
                :key="entry.id"
                class="dash-hlog"
                :class="`tone-${entry.tone}`"
              >
                <span class="dash-hlog__time">[{{ entry.time }}]</span>
                <span class="dash-hlog__tag">[{{ entry.tag }}]</span>
                <span class="dash-hlog__msg">{{ entry.message }}</span>
              </div>
            </div>
          </div>
        </div>

        <div class="dash-agent-card is-on">
          <div class="dash-run">
            <span class="dash-dot is-on" />
            <div class="dash-run__body">
              <b>值守 Agent · AI 客服</b>
              <span>云端值守中</span>
            </div>
            <el-button size="small" text @click="router.push('/xhs/auto-ops')">去自动运营</el-button>
          </div>
          <div class="dash-run-log" aria-hidden="true">
            <div class="dash-run-log__head">
              <span class="dash-run-log__dot" />
              <span class="dash-run-log__title">值守 Agent · 执行日志</span>
            </div>
            <div ref="dutyLogEl" class="dash-run-log__body">
              <div
                v-for="entry in dutyLogs"
                :key="entry.id"
                class="dash-hlog"
                :class="`tone-${entry.tone}`"
              >
                <span class="dash-hlog__time">[{{ entry.time }}]</span>
                <span class="dash-hlog__tag">[{{ entry.tag }}]</span>
                <span class="dash-hlog__msg">{{ entry.message }}</span>
              </div>
            </div>
          </div>
        </div>

        <div class="dash-agent-card is-on">
          <div class="dash-run">
            <span class="dash-dot is-on" />
            <div class="dash-run__body">
              <b>拓客 Agent · AI 获客</b>
              <span>云端拓客中</span>
            </div>
            <el-button size="small" text @click="router.push('/xhs/auto-ops')">去自动运营</el-button>
          </div>
          <div class="dash-run-log" aria-hidden="true">
            <div class="dash-run-log__head">
              <span class="dash-run-log__dot" />
              <span class="dash-run-log__title">拓客 Agent · 执行日志</span>
            </div>
            <div ref="scoutLogEl" class="dash-run-log__body">
              <div
                v-for="entry in scoutLogs"
                :key="entry.id"
                class="dash-hlog"
                :class="`tone-${entry.tone}`"
              >
                <span class="dash-hlog__time">[{{ entry.time }}]</span>
                <span class="dash-hlog__tag">[{{ entry.tag }}]</span>
                <span class="dash-hlog__msg">{{ entry.message }}</span>
              </div>
            </div>
          </div>
        </div>
        <div class="dash-quota">
          竞品配额：笔记 {{ competitors.length }}/20 · 关注用户 {{ watchedCount }}/30
          <template v-if="unreadEvents"> · 未读 {{ unreadEvents }}</template>
        </div>
      </div>
    </section>

    <el-dialog
      v-model="configDialogOpen"
      title="今日最火 · 配置"
      width="420px"
      destroy-on-close
    >
      <el-form label-width="96px" size="default" @submit.prevent>
        <p class="dash-config-hint">
          DeepSeek 会根据备注与行业，先产出搜索词，再用 PC 搜索账号按最热拉取。
        </p>
        <el-form-item label="用途备注">
          <el-input
            v-model="remarkDraft"
            placeholder="这段洞察干什么，如美妆竞品情报"
            clearable
            maxlength="80"
            show-word-limit
          />
        </el-form-item>
        <el-form-item label="行业方向">
          <el-input
            v-model="keywordDraft"
            placeholder="行业/方向，如美妆护肤（与备注至少填一项）"
            clearable
            maxlength="40"
            show-word-limit
            @keyup.enter="saveConfig"
          />
        </el-form-item>
        <el-form-item label="执行间隔">
          <div class="dash-interval-row">
            <el-input-number v-model="intervalDraft" :min="15" :max="1440" :step="5" />
            <span class="dash-interval-unit">分钟</span>
          </div>
          <p class="dash-interval-tip">仅控制「今日最火」后台刷新，与竞品监控间隔独立；最小 15 分钟。</p>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="configDialogOpen = false">取消</el-button>
        <el-button type="primary" :loading="configSaving" @click="saveConfig">保存</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.xhs-dash {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 12px 14px 24px;
  box-sizing: border-box;
  min-height: 100%;
  height: auto;
  overflow: visible;
}
.dash-row--health-hot {
  display: grid;
  grid-template-columns: minmax(260px, 0.85fr) minmax(300px, 1.15fr);
  gap: 14px;
  align-items: stretch;
  flex: 0 0 auto;
}
.dash-row--health-hot > .dash-card {
  min-width: 0;
  display: flex;
  flex-direction: column;
  height: auto;
}
.dash-row--health-hot > .dash-card :deep(.mf-ops__panel-body) {
  flex: 1;
  min-height: 528px;
  height: 528px;
  overflow: hidden;
}
/* 健康区 / 今日最火 同高；仅账号卡片列表滚动 */
.dash-row--health-hot > .dash-card:first-child :deep(.mf-ops__panel-body.dash-health),
.dash-row--health-hot > .dash-card:last-child :deep(.mf-ops__panel-body) {
  height: 528px;
  min-height: 528px;
  overflow: hidden;
}
.dash-card :deep(.mf-ops__panel-head) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.dash-sub {
  margin-left: 10px;
  color: #909399;
  font-size: 12px;
  font-weight: 400;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: min(360px, 42vw);
  display: inline-block;
  vertical-align: bottom;
}
.dash-config-hint {
  margin: 0 0 14px;
  padding: 8px 10px;
  font-size: 12px;
  line-height: 1.5;
  color: #606266;
  background: #f5f7fa;
  border-radius: 6px;
}
.dash-interval-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.dash-interval-unit {
  font-size: 13px;
  color: #606266;
}
.dash-interval-tip {
  margin: 6px 0 0;
  font-size: 12px;
  line-height: 1.4;
  color: #909399;
}
.dash-card :deep(.mf-ops__panel-head) > div:first-child {
  min-width: 0;
  overflow: hidden;
}
.dash-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}
.dash-health {
  display: flex;
  flex-direction: column;
  gap: 8px;
  overflow: hidden;
  height: 100%;
  box-sizing: border-box;
}
.dash-health__top {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 14px;
  align-items: start;
  /* 不随日志伸缩，避免日志抢走账号卡片高度 */
  flex: 0 0 auto;
  min-height: auto;
  overflow: visible;
  position: relative;
  z-index: 2;
}
.dash-health__main {
  min-width: 0;
  max-width: 100%;
  height: auto;
  max-height: 300px;
  overflow-x: hidden;
  overflow-y: auto;
  padding-right: 2px;
  scrollbar-gutter: stable;
}
.dash-health-log {
  /* 只吃卡片+环形图下方的剩余高度，底边贴合面板 */
  flex: 1 1 0;
  min-height: 0;
  height: auto;
  display: flex;
  flex-direction: column;
  border-radius: 8px;
  overflow: hidden;
  position: relative;
  z-index: 0;
  border: 1px solid #e4e7ed;
  background: linear-gradient(180deg, #1c2230 0%, #141821 100%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
}
.dash-health-log__head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 10px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  background: rgba(255, 255, 255, 0.03);
}
.dash-health-log__dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #67c23a;
  box-shadow: 0 0 0 0 rgba(103, 194, 58, 0.55);
  animation: dash-health-pulse 1.8s ease-out infinite;
  flex-shrink: 0;
}
@keyframes dash-health-pulse {
  0% {
    box-shadow: 0 0 0 0 rgba(103, 194, 58, 0.5);
  }
  70% {
    box-shadow: 0 0 0 6px rgba(103, 194, 58, 0);
  }
  100% {
    box-shadow: 0 0 0 0 rgba(103, 194, 58, 0);
  }
}
.dash-health-log__title {
  font-size: 11px;
  color: #c9d1df;
  font-weight: 600;
  letter-spacing: 0.02em;
}
.dash-health-log__body {
  flex: 1;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  padding: 6px 10px 8px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 11px;
  line-height: 1.55;
  scrollbar-width: thin;
  scrollbar-color: #3a4254 transparent;
}
.dash-health-log__body::-webkit-scrollbar {
  width: 6px;
}
.dash-health-log__body::-webkit-scrollbar-track {
  background: transparent;
}
.dash-health-log__body::-webkit-scrollbar-thumb {
  background: #3a4254;
  border-radius: 999px;
}
.dash-health-log__body::-webkit-scrollbar-thumb:hover {
  background: #525c72;
}
.dash-health-log__body::-webkit-scrollbar-corner {
  background: transparent;
}
.dash-hlog {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
  padding: 1px 0;
  color: #c9d1df;
  animation: dash-hlog-in 0.28s ease-out;
}
@keyframes dash-hlog-in {
  from {
    opacity: 0;
    transform: translateY(4px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}
.dash-hlog__time {
  color: #6b7385;
  flex-shrink: 0;
}
.dash-hlog__tag {
  font-weight: 600;
  flex-shrink: 0;
  color: #e5e7eb;
}
.dash-hlog__msg {
  color: #d7dde8;
  word-break: break-word;
}
.dash-hlog.tone-step .dash-hlog__tag {
  color: #60a5fa;
}
.dash-hlog.tone-cyan .dash-hlog__tag {
  color: #22d3ee;
}
.dash-hlog.tone-gold .dash-hlog__tag {
  color: #fbbf24;
}
.dash-hlog.tone-ok .dash-hlog__tag,
.dash-hlog.tone-ok .dash-hlog__msg {
  color: #4ade80;
}
.dash-hlog.tone-warn .dash-hlog__tag {
  color: #fb923c;
}
.dash-hlog.tone-neutral .dash-hlog__tag {
  color: #e5e7eb;
}
.dash-acc-grid {
  grid-template-columns: 1fr !important;
  gap: 6px !important;
  width: 100%;
  max-width: 100%;
}
.dash-acc-grid .mf-ops__card {
  margin: 0;
  padding: 8px 10px;
  border-radius: 8px;
  min-width: 0;
  max-width: 100%;
  width: 100%;
  box-sizing: border-box;
  overflow: hidden;
}
.dash-acc-grid :deep(.mf-ops__card-head) {
  gap: 6px;
  margin-bottom: 6px;
  align-items: center;
}
.dash-acc-grid :deep(.mf-ops__card-avatar) {
  width: 28px;
  height: 28px;
  font-size: 12px;
}
.dash-acc-grid :deep(.mf-ops__card-meta) {
  min-width: 0;
  flex: 1;
}
.dash-acc-grid :deep(.mf-ops__card-meta strong) {
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.dash-acc-grid :deep(.mf-ops__card-meta em) {
  font-size: 10px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.dash-acc-grid :deep(.el-tag) {
  flex-shrink: 0;
  transform: scale(0.92);
  transform-origin: right center;
}
.dash-acc-grid :deep(.mf-ops__card-stats) {
  gap: 2px 8px;
  margin-bottom: 0;
  grid-template-columns: repeat(auto-fit, minmax(64px, 1fr));
}
.dash-acc-grid :deep(.mf-ops__card-stats span) {
  font-size: 10px;
}
.dash-acc-grid :deep(.mf-ops__card-stats b) {
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 100%;
}
.dash-health__chart {
  width: 100%;
  min-height: 120px;
  max-height: none;
  justify-self: stretch;
  overflow: hidden;
  position: relative;
  z-index: 2;
  pointer-events: auto;
  isolation: isolate;
  box-sizing: border-box;
}
.dash-runners {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 12px;
  align-items: stretch;
}
.dash-runners > .dash-quota {
  grid-column: 1 / -1;
}
.dash-agent-card {
  display: flex;
  flex-direction: column;
  height: 440px;
  min-height: 440px;
  max-height: 440px;
  border-radius: 10px;
  border: 1px solid #ebeef5;
  background: #fff;
  overflow: hidden;
  min-width: 0;
}
.dash-agent-card.is-on {
  border-color: #b3e19d;
}
.dash-agent-card .dash-run {
  flex: 0 0 auto;
  border-radius: 0;
  background: #fafafa;
  border-bottom: 1px solid #ebeef5;
  flex-wrap: wrap;
  row-gap: 6px;
}
.dash-run-log {
  flex: 1 1 auto;
  min-height: 0;
  height: auto;
  display: flex;
  flex-direction: column;
  background: linear-gradient(180deg, #1c2230 0%, #141821 100%);
}
.dash-run-log__head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 10px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  background: rgba(255, 255, 255, 0.03);
  flex-shrink: 0;
}
.dash-run-log__dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #67c23a;
  box-shadow: 0 0 0 0 rgba(103, 194, 58, 0.45);
  animation: dash-health-pulse 1.8s ease-out infinite;
  flex-shrink: 0;
}
.dash-run-log__title {
  font-size: 11px;
  color: #c9d1df;
  font-weight: 600;
}
.dash-run-log__body {
  flex: 1;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  padding: 6px 10px 8px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 11px;
  line-height: 1.55;
  scrollbar-width: thin;
  scrollbar-color: #3a4254 transparent;
}
.dash-run-log__body::-webkit-scrollbar {
  width: 6px;
}
.dash-run-log__body::-webkit-scrollbar-track {
  background: transparent;
}
.dash-run-log__body::-webkit-scrollbar-thumb {
  background: #3a4254;
  border-radius: 999px;
}
.dash-run-log__body::-webkit-scrollbar-thumb:hover {
  background: #525c72;
}
.dash-run-log__body::-webkit-scrollbar-corner {
  background: transparent;
}
.dash-run {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border-radius: 8px;
  background: #fafafa;
}
.dash-run--minor {
  opacity: 0.92;
}
.dash-run__body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 12px;
  color: #606266;
}
.dash-run__body b {
  font-size: 13px;
  color: #303133;
}
.dash-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #c0c4cc;
  flex-shrink: 0;
}
.dash-dot.is-on {
  background: #67c23a;
  box-shadow: 0 0 0 3px rgba(103, 194, 58, 0.2);
}
.dash-dot.is-err {
  background: #f56c6c;
}
.dash-quota {
  margin-top: 4px;
  font-size: 12px;
  color: #909399;
}
.dash-empty {
  color: #909399;
  font-size: 13px;
  padding: 12px 0;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px;
}
.dash-inline-icon {
  vertical-align: middle;
  margin: 0 2px;
}
.dash-hot-body {
  padding-top: 8px !important;
}
.dash-hot-split {
  display: grid;
  grid-template-columns: minmax(0, 1.35fr) minmax(160px, 0.95fr);
  gap: 12px;
  height: 100%;
  min-height: 0;
}
.dash-hot-left {
  min-width: 0;
  height: 100%;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.dash-hot-left__posts {
  /* 不撑满剩余高度，避免末条卡片与运营建议之间出现大块空白 */
  flex: 0 1 auto;
  min-height: 0;
  max-height: calc(100% - 116px);
  overflow-x: hidden;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
}
.dash-hot-left__empty {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-wrap: wrap;
  gap: 4px;
  color: #909399;
  font-size: 12px;
  text-align: center;
  padding: 8px;
}
.dash-hot-grid {
  display: grid;
  grid-template-columns: 1fr;
  gap: 6px;
}
.dash-hot {
  display: flex;
  gap: 8px;
  padding: 6px 8px;
  border-radius: 8px;
  border: 1px solid #ebeef5;
  text-decoration: none;
  color: inherit;
  background: #fff;
}
.dash-hot img,
.dash-hot__ph {
  width: 40px;
  height: 52px;
  object-fit: cover;
  border-radius: 5px;
  background: #f2f3f5;
  flex-shrink: 0;
}
.dash-hot__body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 11px;
  color: #606266;
  min-width: 0;
}
.dash-hot__body b {
  color: #303133;
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
}
.dash-advice-type {
  flex: 1 1 0;
  min-height: 96px;
  height: auto;
  display: flex;
  flex-direction: column;
  border-radius: 8px;
  border: 1px solid #e4e7ed;
  background: #f8fafc;
  overflow: hidden;
  transition: border-color 0.2s ease, box-shadow 0.2s ease;
  user-select: text;
}
.dash-advice-type.is-hold-paused {
  border-color: #a0cfff;
  box-shadow: 0 0 0 1px rgba(64, 158, 255, 0.15);
}
.dash-advice-type__label {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 5px 8px 5px 10px;
  font-size: 11px;
  font-weight: 600;
  color: #606266;
  border-bottom: 1px solid #ebeef5;
  background: #fff;
}
.dash-advice-type__actions {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}
.dash-advice-type__icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  padding: 0;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: #909399;
  cursor: pointer;
  flex-shrink: 0;
}
.dash-advice-type__icon-btn:hover:not(:disabled) {
  color: #409eff;
  background: #ecf5ff;
}
.dash-advice-type__icon-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.dash-advice-type__body {
  flex: 1;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  padding: 8px 10px;
  font-size: 12px;
  line-height: 1.55;
  color: #303133;
  white-space: pre-wrap;
  word-break: break-word;
}
.dash-advice-type__body.is-placeholder {
  color: #909399;
}
.dash-advice-type__caret {
  display: inline-block;
  width: 1.5px;
  height: 0.95em;
  margin-left: 1px;
  vertical-align: -0.12em;
  background: #409eff;
}
.dash-advice-type__caret.blink {
  animation: dash-caret-blink 1s step-end infinite;
}
@keyframes dash-caret-blink {
  50% {
    opacity: 0;
  }
}

/* 云端 Agent 舱：上 tip / 下日志 */
.dash-agent {
  position: relative;
  border-radius: 10px;
  overflow: hidden;
  min-height: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: stretch;
  justify-content: flex-start;
  gap: 8px;
  padding: 0;
  background: transparent;
  border: none;
}
.dash-agent__tip {
  position: relative;
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 12px 8px 10px;
  border-radius: 10px;
  overflow: hidden;
  background: linear-gradient(165deg, #f0f7ff 0%, #eef2ff 45%, #f8fafc 100%);
  border: 1px solid #e4e7ed;
  min-height: 132px;
}
.dash-agent__sky {
  position: absolute;
  inset: 0;
  pointer-events: none;
  overflow: hidden;
}
.dash-agent-log {
  flex: 1 1 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  border-radius: 8px;
  overflow: hidden;
  border: 1px solid #e4e7ed;
  background: linear-gradient(180deg, #1c2230 0%, #141821 100%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
}
.dash-agent-log__head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 10px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  background: rgba(255, 255, 255, 0.03);
  flex-shrink: 0;
}
.dash-agent-log__dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #409eff;
  box-shadow: 0 0 0 0 rgba(64, 158, 255, 0.55);
  animation: dash-health-pulse 1.8s ease-out infinite;
  flex-shrink: 0;
}
.dash-agent-log__title {
  font-size: 11px;
  color: #c9d1df;
  font-weight: 600;
  letter-spacing: 0.02em;
}
.dash-agent-log__body {
  flex: 1;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  padding: 6px 10px 8px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 11px;
  line-height: 1.55;
  scrollbar-width: thin;
  scrollbar-color: #3a4254 transparent;
}
.dash-agent-log__body::-webkit-scrollbar {
  width: 6px;
}
.dash-agent-log__body::-webkit-scrollbar-track {
  background: transparent;
}
.dash-agent-log__body::-webkit-scrollbar-thumb {
  background: #3a4254;
  border-radius: 999px;
}
.dash-agent-log__body::-webkit-scrollbar-thumb:hover {
  background: #525c72;
}
.dash-agent-log__body::-webkit-scrollbar-corner {
  background: transparent;
}
.dash-agent__cloud {
  position: absolute;
  width: 48px;
  height: 16px;
  border-radius: 20px;
  background: rgba(255, 255, 255, 0.75);
  box-shadow: 18px 0 0 -2px rgba(255, 255, 255, 0.55);
  opacity: 0.7;
  animation: dash-cloud 12s linear infinite;
}
.dash-agent__cloud.c1 {
  top: 18%;
  left: -20%;
  animation-duration: 14s;
}
.dash-agent__cloud.c2 {
  top: 42%;
  left: -30%;
  width: 36px;
  animation-duration: 18s;
  animation-delay: -4s;
}
.dash-agent__cloud.c3 {
  top: 68%;
  left: -15%;
  width: 42px;
  animation-duration: 16s;
  animation-delay: -8s;
}
.dash-agent__dot {
  position: absolute;
  width: 3px;
  height: 3px;
  border-radius: 50%;
  background: #79bbff;
  left: calc(12% + var(--i) * 12%);
  bottom: 8%;
  opacity: 0;
  animation: dash-up 2.8s ease-in-out infinite;
  animation-delay: calc(var(--i) * 0.35s);
}
.dash-agent__core {
  position: relative;
  width: 72px;
  height: 72px;
  z-index: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
.dash-agent__ring {
  position: absolute;
  inset: 10px;
  border-radius: 50%;
  border: 2px solid rgba(64, 158, 255, 0.35);
  border-top-color: #409eff;
  animation: dash-spin 2.4s linear infinite;
}
.dash-agent__ring--outer {
  inset: 0;
  border-width: 1px;
  border-color: rgba(64, 158, 255, 0.15);
  border-right-color: rgba(103, 194, 58, 0.55);
  animation-duration: 4.5s;
  animation-direction: reverse;
}
.dash-agent__face {
  width: 36px;
  height: 36px;
  border-radius: 12px;
  background: linear-gradient(145deg, #409eff, #79bbff);
  box-shadow: 0 6px 16px rgba(64, 158, 255, 0.35);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  z-index: 1;
}
.dash-agent__eye {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: #fff;
  animation: dash-blink 3.2s ease-in-out infinite;
}
.dash-agent__copy {
  position: relative;
  z-index: 1;
  text-align: center;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 0 4px;
}
.dash-agent__copy b {
  font-size: 13px;
  color: #303133;
}
.dash-agent__copy span {
  font-size: 11px;
  color: #909399;
  line-height: 1.4;
}
.dash-agent.is-running .dash-agent__ring {
  animation-duration: 0.9s;
  border-top-color: #67c23a;
}
.dash-agent.is-running .dash-agent__dot {
  opacity: 1;
}
.dash-agent.is-running .dash-agent__face {
  background: linear-gradient(145deg, #67c23a, #95d475);
  box-shadow: 0 6px 16px rgba(103, 194, 58, 0.35);
}
.dash-agent.is-running .dash-agent-log__dot {
  background: #67c23a;
}
.dash-agent.is-error .dash-agent__ring {
  border-top-color: #e6a23c;
  animation-duration: 3s;
}
.dash-agent.is-error .dash-agent__face {
  background: linear-gradient(145deg, #e6a23c, #f3d19e);
  box-shadow: 0 6px 16px rgba(230, 162, 60, 0.35);
}
.dash-agent.is-standby .dash-agent__face {
  animation: dash-breathe 3.2s ease-in-out infinite;
}
.dash-agent.is-need_config .dash-agent__ring,
.dash-agent.is-need_account .dash-agent__ring {
  animation-duration: 6s;
  border-top-color: #a0cfff;
}

@keyframes dash-spin {
  to {
    transform: rotate(360deg);
  }
}
@keyframes dash-cloud {
  to {
    transform: translateX(160%);
  }
}
@keyframes dash-up {
  0% {
    transform: translateY(0);
    opacity: 0;
  }
  30% {
    opacity: 0.85;
  }
  100% {
    transform: translateY(-72px);
    opacity: 0;
  }
}
@keyframes dash-blink {
  0%,
  92%,
  100% {
    transform: scaleY(1);
  }
  96% {
    transform: scaleY(0.15);
  }
}
@keyframes dash-breathe {
  0%,
  100% {
    transform: scale(1);
  }
  50% {
    transform: scale(1.05);
  }
}
.xhs-dash > .dash-card {
  flex: 0 0 auto;
}
.dash-growth {
  display: grid;
  grid-template-columns: 280px 1fr;
  gap: 14px;
  align-items: start;
  min-height: 340px;
  height: auto;
  overflow: visible;
}
.dash-growth__list {
  min-width: 0;
  overflow: visible;
}
.dash-growth__chart-wrap {
  min-width: 0;
  min-height: 340px;
}
.dash-growth__item {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  width: 100%;
  text-align: left;
  border: 1px solid #ebeef5;
  background: #fff;
  border-radius: 8px;
  padding: 8px 10px;
  margin-bottom: 8px;
  cursor: pointer;
  font-size: 12px;
  color: #606266;
}
.dash-growth__item b {
  color: #303133;
  font-size: 13px;
}
.dash-growth__item.active {
  border-color: #409eff;
  background: #ecf5ff;
}
.dash-chart-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 8px;
}
.dash-chart-toolbar .dash-sub {
  max-width: none;
  flex: 1;
  min-width: 0;
}
.dash-trend-chart {
  width: 100%;
  height: 300px;
}
@media (max-width: 1100px) {
  .dash-row--health-hot {
    grid-template-columns: 1fr;
  }
  .dash-runners {
    grid-template-columns: 1fr;
  }
  .dash-agent-card {
    height: 400px;
    min-height: 400px;
    max-height: 400px;
  }
}
@media (max-width: 960px) {
  .dash-health__top,
  .dash-growth {
    grid-template-columns: 1fr;
  }
  .dash-health__chart {
    min-height: 120px;
  }
  .dash-health__main {
    max-height: none;
  }
  .dash-row--health-hot > .dash-card:first-child :deep(.mf-ops__panel-body.dash-health),
  .dash-row--health-hot > .dash-card:last-child :deep(.mf-ops__panel-body) {
    height: auto;
    min-height: 462px;
  }
  .dash-hot-split {
    grid-template-columns: 1fr;
  }
  .dash-agent {
    min-height: 260px;
  }
}
</style>
