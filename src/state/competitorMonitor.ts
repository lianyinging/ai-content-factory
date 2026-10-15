import { computed, ref, type Ref } from 'vue'
import { ElMessage, ElNotification } from 'element-plus'
import {
  competitorList,
  watchedUserList,
  type CompetitorRow,
  type WatchedUserRow,
} from '@/api/competitors'
import {
  isDueAt,
  syncCompetitorNote,
  syncWatchedUser,
} from '@/api/competitorSync'
import router from '@/router'

const STORAGE_KEY = 'ai-factory.competitorMonitor.v1'
const CTL_KEY = '__competitorMonitorCtl_v1__'
/** 轮询检查基础间隔；实际用 jitter 错开，避免整点齐刷 */
const TICK_BASE_MS = 50_000
const TICK_JITTER_MS = 25_000
/**
 * 单条执行前的拟人等待（思考/翻页感）。
 * 到期笔记每轮最多刷 1 条，靠轮询间隔自然错开，绝不并发齐发。
 */
const PRE_DELAY_MIN_MS = 8_000
const PRE_DELAY_MAX_MS = 28_000

type Ctl = {
  running: Ref<boolean>
  ticking: boolean
  timer: ReturnType<typeof setTimeout> | null
  currentLabel: Ref<string>
  lastTickAt: Ref<number>
  lastError: Ref<string>
  lastOkAt: Ref<number>
  refreshedCount: Ref<number>
}

type Persisted = { running: boolean }

function ctl(): Ctl {
  const g = globalThis as unknown as Record<string, Ctl>
  if (!g[CTL_KEY]) {
    g[CTL_KEY] = {
      running: ref(false),
      ticking: false,
      timer: null,
      currentLabel: ref(''),
      lastTickAt: ref(0),
      lastError: ref(''),
      lastOkAt: ref(0),
      refreshedCount: ref(0),
    }
  }
  return g[CTL_KEY]
}

function readPersisted(): Persisted {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return { running: false }
    const j = JSON.parse(raw) as Persisted
    return { running: Boolean(j?.running) }
  } catch {
    return { running: false }
  }
}

function writePersisted(running: boolean) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ running } satisfies Persisted))
  } catch {
    /* ignore */
  }
}

function sleep(ms: number) {
  return new Promise<void>((r) => setTimeout(r, ms))
}

function randInt(min: number, max: number) {
  return min + Math.floor(Math.random() * (max - min + 1))
}

function nextTickDelayMs() {
  return TICK_BASE_MS + randInt(0, TICK_JITTER_MS)
}

function preActionDelayMs() {
  return randInt(PRE_DELAY_MIN_MS, PRE_DELAY_MAX_MS)
}

/** Fisher–Yates，打乱到期队列，避免固定顺序像脚本 */
function shuffleInPlace<T>(arr: T[]): T[] {
  for (let i = arr.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1))
    const t = arr[i]!
    arr[i] = arr[j]!
    arr[j] = t
  }
  return arr
}

function notifyRisk(message: string) {
  ElNotification({
    title: '竞品监控已暂停',
    message: message || '触发平台风控，请完成验证后重新启动监控',
    type: 'warning',
    duration: 10000,
    onClick: () => {
      if (router.currentRoute.value.path !== '/xhs/competitors') {
        void router.push('/xhs/competitors')
      }
    },
  })
}

async function tickOnce() {
  const c = ctl()
  if (!c.running.value || c.ticking) return
  c.ticking = true
  c.lastTickAt.value = Date.now()
  try {
    let notes: CompetitorRow[] = []
    let users: WatchedUserRow[] = []
    try {
      notes = await competitorList()
    } catch (e) {
      c.lastError.value = e instanceof Error ? e.message : String(e)
      return
    }
    try {
      users = await watchedUserList()
    } catch {
      users = []
    }

    // 每轮最多处理 1 条到期笔记 + 最多 1 个用户，串行拟人，禁止齐刷
    const dueNotes = shuffleInPlace(notes.filter((n) => isDueAt(n.nextRunAt)))
    const dueUsers = shuffleInPlace(users.filter((u) => isDueAt(u.nextRunAt)))
    const note = dueNotes[0]
    const user = dueUsers[0]

    if (note) {
      if (!c.running.value) return
      c.currentLabel.value = note.title || note.noteId || '笔记'
      await sleep(preActionDelayMs())
      if (!c.running.value) return
      const res = await syncCompetitorNote(note, { reportError: true, trigger: '后台监控' })
      if (res.risk) {
        c.lastError.value = res.error || 'RISK_CONTROL'
        stopInternal(false)
        notifyRisk(res.error || '触发人机验证，监控已自动停止')
        return
      }
      if (res.ok) {
        c.refreshedCount.value += 1
        c.lastOkAt.value = Date.now()
        c.lastError.value = ''
      } else {
        c.lastError.value = res.error || '刷新失败'
      }
    }

    if (user) {
      if (!c.running.value) return
      // 笔记与用户之间再空一档，避免紧挨着打两个接口
      await sleep(preActionDelayMs())
      if (!c.running.value) return
      c.currentLabel.value = user.nickname || user.externalUserId || '用户'
      const res = await syncWatchedUser(user, { reportError: true })
      if (res.risk) {
        c.lastError.value = res.error || 'RISK_CONTROL'
        stopInternal(false)
        notifyRisk(res.error || '触发人机验证，监控已自动停止')
        return
      }
      if (res.ok) {
        c.refreshedCount.value += 1
        c.lastOkAt.value = Date.now()
        c.lastError.value = ''
      } else {
        c.lastError.value = res.error || '同步失败'
      }
    }
  } finally {
    c.currentLabel.value = ''
    c.ticking = false
    // 下一轮用随机间隔续约（setTimeout 链，比固定 setInterval 更不像机器）
    if (c.running.value) scheduleNextTick()
  }
}

function scheduleNextTick() {
  const c = ctl()
  if (c.timer != null) {
    clearTimeout(c.timer)
    c.timer = null
  }
  if (!c.running.value) return
  c.timer = setTimeout(() => {
    c.timer = null
    void tickOnce()
  }, nextTickDelayMs())
}

function ensureTimer() {
  const c = ctl()
  if (c.timer != null || c.ticking) return
  scheduleNextTick()
}

function clearTimer() {
  const c = ctl()
  if (c.timer != null) {
    clearTimeout(c.timer)
    c.timer = null
  }
}

function stopInternal(persist: boolean) {
  const c = ctl()
  c.running.value = false
  c.currentLabel.value = ''
  clearTimer()
  if (persist) writePersisted(false)
}

export function useCompetitorMonitorStore() {
  const c = ctl()

  const running = computed(() => c.running.value)
  const busyLabel = computed(() => c.currentLabel.value)
  const statusText = computed(() => {
    if (!c.running.value) return ''
    if (c.currentLabel.value) return `正在刷新：${c.currentLabel.value}`
    return '竞品监控运行中'
  })

  function start() {
    if (c.running.value) {
      ElMessage.info('监控已在运行')
      return
    }
    c.running.value = true
    writePersisted(true)
    ElMessage.success('已启动后台监控：笔记将错开拟人间隔逐条刷新，切页不停')
    // 启动后先随机等待再跑第一轮，避免一点击就齐刷
    scheduleNextTick()
  }

  function stop() {
    if (!c.running.value) {
      ElMessage.info('监控未在运行')
      return
    }
    stopInternal(true)
    ElMessage.success('已停止后台监控')
  }

  function toggle() {
    if (c.running.value) stop()
    else start()
  }

  /** App 启动时恢复上次运行状态（挂在 globalThis，切页不丢） */
  function resumeIfNeeded() {
    const p = readPersisted()
    if (!p.running) return
    if (c.running.value) {
      ensureTimer()
      return
    }
    c.running.value = true
    scheduleNextTick()
  }

  return {
    running,
    busyLabel,
    statusText,
    lastTickAt: c.lastTickAt,
    lastError: c.lastError,
    lastOkAt: c.lastOkAt,
    refreshedCount: c.refreshedCount,
    start,
    stop,
    toggle,
    resumeIfNeeded,
  }
}
