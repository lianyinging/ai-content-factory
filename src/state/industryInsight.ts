import { computed, ref, type Ref } from 'vue'
import { ElNotification } from 'element-plus'
import { authList, itemSearch, reportCatbusError } from '@/api/catbus'
import { aiHotSearchQuery, aiViralAnalyze } from '@/api/ai'
import {
  dashboardInsightConfigGet,
  industryInsightRunAdd,
  industryInsightRunLatest,
  parseAnalysis,
  parseHotPosts,
  type DashboardInsightConfig,
  type HotPost,
  type InsightAnalysis,
  type IndustryInsightRun,
} from '@/api/dashboardInsight'

const CTL_KEY = '__industryInsightCtl_v2__'
const DEFAULT_INTERVAL_MIN = 30
const PLATFORM = 'xhs'

function clampIntervalMinutes(raw: unknown): number {
  const n = Math.round(Number(raw))
  if (!Number.isFinite(n)) return DEFAULT_INTERVAL_MIN
  return Math.min(1440, Math.max(15, n))
}

function tickMsFromConfig(cfg: DashboardInsightConfig | null | undefined): number {
  return clampIntervalMinutes(cfg?.intervalMinutes) * 60 * 1000
}

type Ctl = {
  running: Ref<boolean>
  ticking: boolean
  timer: ReturnType<typeof setTimeout> | null
  lastOkAt: Ref<number>
  lastError: Ref<string>
  lastRun: Ref<IndustryInsightRun | null>
  posts: Ref<HotPost[]>
  analysis: Ref<InsightAnalysis | null>
  config: Ref<DashboardInsightConfig | null>
  searchAccount: Ref<string>
  searchNickname: Ref<string>
  /** DeepSeek 产出的实际搜索词 */
  searchQuery: Ref<string>
  /** running 子阶段：expand=出词 / search=搜帖 / analyze=写作建议 */
  phase: Ref<'idle' | 'expand' | 'search' | 'analyze'>
}

function ctl(): Ctl {
  const g = globalThis as unknown as Record<string, Ctl>
  if (!g[CTL_KEY]) {
    g[CTL_KEY] = {
      running: ref(false),
      ticking: false,
      timer: null,
      lastOkAt: ref(0),
      lastError: ref(''),
      lastRun: ref(null),
      posts: ref([]),
      analysis: ref(null),
      config: ref(null),
      searchAccount: ref(''),
      searchNickname: ref(''),
      searchQuery: ref(''),
      phase: ref('idle'),
    }
  }
  return g[CTL_KEY]
}

async function resolveSearchAccount(): Promise<{ account: string; nickname: string } | null> {
  try {
    const env = await authList(PLATFORM)
    const data = env.data
    const list: unknown[] = Array.isArray(data)
      ? data
      : data && typeof data === 'object' && Array.isArray((data as { accounts?: unknown[] }).accounts)
        ? (data as { accounts: unknown[] }).accounts
        : []
    for (const x of list) {
      if (!x || typeof x !== 'object') continue
      const r = x as Record<string, unknown>
      const user =
        r.user && typeof r.user === 'object' ? (r.user as Record<string, unknown>) : {}
      const acct = String(r.account || r.name || '').trim()
      if (!acct) continue
      const loggedIn =
        r.logged_in === true || r.loggedIn === true || Boolean(user.id || user.uid)
      if (!loggedIn) continue
      return {
        account: acct,
        nickname: String(user.nickname || user.name || r.nickname || acct),
      }
    }
    // fallback: first account even if login flag unclear
    for (const x of list) {
      if (!x || typeof x !== 'object') continue
      const r = x as Record<string, unknown>
      const user =
        r.user && typeof r.user === 'object' ? (r.user as Record<string, unknown>) : {}
      const acct = String(r.account || r.name || '').trim()
      if (!acct) continue
      return {
        account: acct,
        nickname: String(user.nickname || user.name || r.nickname || acct),
      }
    }
  } catch {
    /* ignore */
  }
  return null
}

function applyRun(run: IndustryInsightRun | null) {
  const c = ctl()
  c.lastRun.value = run
  if (!run) {
    c.posts.value = []
    c.analysis.value = null
    return
  }
  c.posts.value = parseHotPosts(run.postsJson)
  c.analysis.value = parseAnalysis(run.analysisJson)
  if (run.status === 'ok') {
    c.lastError.value = ''
    c.lastOkAt.value = Date.now()
  } else if (run.errorMessage) {
    c.lastError.value = run.errorMessage
  }
}

export async function loadIndustryInsightFromDb() {
  const c = ctl()
  try {
    c.config.value = await dashboardInsightConfigGet(PLATFORM)
  } catch {
    c.config.value = {
      platform: PLATFORM,
      remark: '',
      industryKeyword: '',
      intervalMinutes: DEFAULT_INTERVAL_MIN,
      updatedAt: '',
    }
  }
  try {
    const run = await industryInsightRunLatest(PLATFORM)
    applyRun(run)
  } catch {
    applyRun(null)
  }
  const acct = await resolveSearchAccount()
  c.searchAccount.value = acct?.account || ''
  c.searchNickname.value = acct?.nickname || ''
}

async function runOnce(opts?: { notifyRisk?: boolean }): Promise<boolean> {
  const c = ctl()
  if (c.ticking) return false
  c.ticking = true
  c.running.value = true
  c.phase.value = 'expand'
  try {
    const cfg = await dashboardInsightConfigGet(PLATFORM)
    c.config.value = cfg
    const industry = (cfg.industryKeyword || '').trim()
    const remark = (cfg.remark || '').trim()
    if (!industry && !remark) {
      c.lastError.value = '请先保存行业关键词或用途备注'
      return false
    }
    const acct = await resolveSearchAccount()
    c.searchAccount.value = acct?.account || ''
    c.searchNickname.value = acct?.nickname || ''
    if (!acct?.account) {
      c.lastError.value = '请先绑定并登录 PC 搜索账号'
      return false
    }

    // 备注/行业 → DeepSeek 产出搜索词
    const searchQuery = await aiHotSearchQuery({
      remark,
      industryKeyword: industry || remark,
    })
    const query = (searchQuery || industry || remark).trim()
    c.searchQuery.value = query
    if (!query) {
      c.lastError.value = '未能生成搜索关键词'
      return false
    }

    c.phase.value = 'search'
    const res = await itemSearch(PLATFORM, {
      query,
      sort: 'popular',
      account: acct.account,
      limit: 5,
    })
    if (!res.ok) {
      const err = res.error || '搜索失败'
      const isRisk = /RISK_CONTROL|461|471|captcha|rate_limit|blocked/i.test(
        `${err} ${JSON.stringify(res.envelope || {})}`,
      )
      await industryInsightRunAdd({
        platform: PLATFORM,
        account: acct.account,
        keyword: query,
        postsJson: '[]',
        analysisJson: '',
        status: isRisk ? 'risk' : 'error',
        errorMessage: err,
      })
      c.lastError.value = err
      if (opts?.notifyRisk !== false) {
        void reportCatbusError(res.envelope, err)
      } else if (isRisk) {
        ElNotification({
          title: '今日最火刷新遇风控',
          message: '已跳过本次，稍后自动重试',
          type: 'warning',
          duration: 8000,
        })
      }
      const latest = await industryInsightRunLatest(PLATFORM)
      applyRun(latest)
      return false
    }

    const items = (res.items || []).slice(0, 5)
    const posts: HotPost[] = items.map((it) => ({
      id: it.id,
      title: it.title,
      authorName: it.authorName,
      coverUrl: it.coverUrl,
      url: it.url,
      likes: it.likes,
      collects: it.collects,
      comments: it.comments,
      shares: it.shares,
      desc: it.desc,
    }))

    c.phase.value = 'analyze'
    let analysis: InsightAnalysis = {
      summary: '',
      hooks: [],
      structure: '',
      suggestions: [],
      score: 0,
    }
    try {
      analysis = await aiViralAnalyze({
        scope: 'group',
        platform: PLATFORM,
        items: posts.map((p) => ({
          title: p.title,
          authorName: p.authorName,
          likes: p.likes,
          collects: p.collects,
          comments: p.comments,
          shares: p.shares,
          desc: p.desc,
        })),
      })
      if (!analysis.suggestions.length && analysis.summary) {
        analysis.suggestions = [
          '参考热帖标题钩子与封面反差，先写清目标人群痛点再展开干货。',
          '正文用清单/步骤结构，结尾留一个可评论的互动问题。',
        ]
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      analysis.summary = `热帖已拉取，AI 分析暂不可用：${msg}`
    }

    const run = await industryInsightRunAdd({
      platform: PLATFORM,
      account: acct.account,
      keyword: query,
      postsJson: JSON.stringify(posts),
      analysisJson: JSON.stringify(analysis),
      status: 'ok',
      errorMessage: '',
    })
    applyRun(run)
    c.lastError.value = ''
    return true
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    c.lastError.value = msg
    return false
  } finally {
    c.ticking = false
    c.running.value = false
    c.phase.value = 'idle'
  }
}

function clearTimer() {
  const c = ctl()
  if (c.timer) {
    clearTimeout(c.timer)
    c.timer = null
  }
}

function scheduleNext() {
  const c = ctl()
  clearTimer()
  const ms = tickMsFromConfig(c.config.value)
  c.timer = setTimeout(() => {
    void tick()
  }, ms)
}

async function tick() {
  const c = ctl()
  // 有行业词或备注即可自动跑（DeepSeek 会产出搜索词）
  try {
    c.config.value = await dashboardInsightConfigGet(PLATFORM)
  } catch {
    /* ignore */
  }
  const industry = (c.config.value?.industryKeyword || '').trim()
  const remark = (c.config.value?.remark || '').trim()
  if (industry || remark) {
    await runOnce({ notifyRisk: false })
  }
  scheduleNext()
}

let resumed = false

export function useIndustryInsightStore() {
  const c = ctl()

  const statusText = computed(() => {
    if (c.running.value) {
      if (c.phase.value === 'expand') return 'DeepSeek 生成搜索词…'
      if (c.phase.value === 'search') return '搜索最热帖…'
      if (c.phase.value === 'analyze') return 'AI 分析写作建议…'
      return '刷新中…'
    }
    if (c.lastError.value) return c.lastError.value
    if (c.lastOkAt.value) {
      const d = new Date(c.lastOkAt.value)
      return `上次成功 ${d.toLocaleString()}`
    }
    return '空闲'
  })

  async function refreshNow() {
    return runOnce({ notifyRisk: true })
  }

  function resumeIfNeeded() {
    if (resumed) return
    resumed = true
    void loadIndustryInsightFromDb().then(() => {
      // 恢复上次实际搜索词（落在 run.keyword）
      if (c.lastRun.value?.keyword) c.searchQuery.value = c.lastRun.value.keyword
      scheduleNext()
    })
  }

  function setConfigLocal(cfg: DashboardInsightConfig) {
    c.config.value = cfg
    // 间隔变更后立刻按新周期重排下一轮
    scheduleNext()
  }

  return {
    running: computed(() => c.running.value),
    phase: c.phase,
    lastError: c.lastError,
    lastOkAt: c.lastOkAt,
    lastRun: c.lastRun,
    posts: c.posts,
    analysis: c.analysis,
    config: c.config,
    searchAccount: c.searchAccount,
    searchNickname: c.searchNickname,
    searchQuery: c.searchQuery,
    statusText,
    refreshNow,
    resumeIfNeeded,
    loadFromDb: loadIndustryInsightFromDb,
    setConfigLocal,
  }
}
