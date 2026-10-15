<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  Refresh,
  TrendCharts,
  DataAnalysis,
  ChatDotRound,
  Comment,
  Opportunity,
  Clock,
  Delete,
} from '@element-plus/icons-vue'
import * as echarts from 'echarts'
import { openUrl } from '@tauri-apps/plugin-opener'
import { mediaDisplaySrc } from '@/api/assets'
import { catbusRun, reportCatbusError, reportIfCatbusFailed } from '@/api/catbus'
import { syncCompetitorNote } from '@/api/competitorSync'
import { aiCompetitorChat } from '@/api/ai'
import { useCompetitorMonitorStore } from '@/state/competitorMonitor'
import { extractNoteId, noteRefForCatbus } from '@/utils/xhsNoteRef'
import {
  competitorAdd,
  competitorComments,
  competitorDelete,
  competitorDeleteOpportunity,
  competitorGetSettings,
  competitorList,
  competitorListOpportunities,
  competitorPutSettings,
  competitorRefreshLogs,
  competitorScanOpportunities,
  competitorSetChat,
  competitorSetKeywords,
  competitorSnapshots,
  computeAnalysis,
  watchedUserAdd,
  watchedUserDelete,
  watchedUserEvents,
  watchedUserList,
  watchedUserMarkRead,
  watchedUserRefresh,
  type CommentRow,
  type CompetitorRow,
  type OpportunityRow,
  type RefreshLogRow,
  type SnapshotRow,
  type WatchedEventRow,
  type WatchedUserRow,
} from '@/api/competitors'

function coverSrc(row: CompetitorRow): string {
  const local = String(row.coverLocalPath || '').trim()
  const remote = String(row.coverUrl || '').trim()
  return mediaDisplaySrc(local || remote)
}

const MONITOR_LIMIT = 20
const INTERVAL_JITTER = 10

const { running: monitorRunning, toggle: toggleMonitor } = useCompetitorMonitorStore()

const viewTab = ref<'notes' | 'users'>('notes')
const sortBy = ref<'time' | 'score' | 'likes'>('time')
const loading = ref(false)
const refreshingIds = ref<Set<string>>(new Set())

const competitors = ref<CompetitorRow[]>([])
const watchedUsers = ref<WatchedUserRow[]>([])
const selectedUserId = ref('')
const events = ref<WatchedEventRow[]>([])

const noteAddOpen = ref(false)
const noteUrl = ref('')
const userAddOpen = ref(false)
const profileUrl = ref('')

const intervalOpen = ref(false)
const globalKwOpen = ref(false)
const scanInterval = ref(60)
const globalKeywords = ref<string[]>([])
const kwInput = ref('')

const detailOpen = ref(false)
const detailTab = ref('trend')
const current = ref<CompetitorRow | null>(null)
const snaps = ref<SnapshotRow[]>([])
const analysis = ref<ReturnType<typeof computeAnalysis> | null>(null)
const chatInput = ref('')
const chatBusy = ref(false)
const chatMessages = ref<{ role: string; content: string }[]>([])
const opportunities = ref<OpportunityRow[]>([])
const noteKeywords = ref<string[]>([])

const commentsOpen = ref(false)
const comments = ref<CommentRow[]>([])
const logsOpen = ref(false)
const logs = ref<RefreshLogRow[]>([])
const logsPage = ref(1)
const LOG_PAGE_SIZE = 10

const chartEl = ref<HTMLDivElement | null>(null)
let chart: echarts.ECharts | null = null

const pagedLogs = computed(() => {
  const start = (logsPage.value - 1) * LOG_PAGE_SIZE
  return logs.value.slice(start, start + LOG_PAGE_SIZE)
})

const sortedCompetitors = computed(() => {
  const list = [...competitors.value]
  if (sortBy.value === 'likes') list.sort((a, b) => b.likes - a.likes)
  else if (sortBy.value === 'score') list.sort((a, b) => b.analysisScore - a.analysisScore)
  else list.sort((a, b) => (b.updatedAt || '').localeCompare(a.updatedAt || ''))
  return list
})

const selectedUser = computed(
  () => watchedUsers.value.find((u) => u.id === selectedUserId.value) || null,
)

function fmt(n: number) {
  if (n >= 10000) return `${(n / 10000).toFixed(1)}万`
  return String(n ?? 0)
}

function formatDateTime(raw?: string) {
  if (!raw) return ''
  const s = raw.replace('T', ' ').replace('Z', '')
  return s.length > 19 ? s.slice(0, 19) : s
}

function formatLogStatus(raw?: string) {
  const s = String(raw || '').trim().toLowerCase()
  if (s === 'ok' || s === 'success' || s === '成功') return '成功'
  if (s === 'error' || s === 'fail' || s === 'failed' || s === '失败') return '失败'
  return raw || '—'
}

function formatLogMessage(raw?: string) {
  const s = String(raw || '').trim().toLowerCase()
  if (s === 'manual refresh') return '手动刷新'
  if (s === 'auto refresh' || s === 'scheduled refresh') return '定时刷新'
  if (s === 'monitor refresh' || s === 'background refresh') return '后台监控'
  return raw || '—'
}

function logStatusType(raw?: string): 'success' | 'danger' | 'info' {
  const s = String(raw || '').trim().toLowerCase()
  if (s === 'ok' || s === 'success' || s === '成功') return 'success'
  if (s === 'error' || s === 'fail' || s === 'failed' || s === '失败') return 'danger'
  return 'info'
}

function extractUserId(url: string) {
  const m =
    url.match(/user\/profile\/([a-zA-Z0-9]+)/) ||
    url.match(/profile\/([a-zA-Z0-9]+)/) ||
    url.match(/\/u\/([a-zA-Z0-9]+)/)
  return m?.[1] || ''
}

async function openLink(url: string) {
  if (!url) return
  try {
    await openUrl(url)
  } catch {
    window.open(url, '_blank')
  }
}

async function loadCompetitors() {
  loading.value = true
  try {
    competitors.value = await competitorList()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
  }
}

async function loadWatchedUsers() {
  loading.value = true
  try {
    watchedUsers.value = await watchedUserList()
    if (!selectedUserId.value && watchedUsers.value[0]) {
      selectedUserId.value = watchedUsers.value[0].id
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
  }
}

async function loadEvents(userId: string) {
  if (!userId) {
    events.value = []
    return
  }
  try {
    events.value = await watchedUserEvents(userId)
  } catch {
    events.value = []
  }
}

async function loadSettings() {
  try {
    const s = await competitorGetSettings()
    scanInterval.value = s.scanIntervalMinutes || 60
    globalKeywords.value = Array.isArray(s.keywords) ? s.keywords.map(String) : []
  } catch {
    /* ignore */
  }
}

function parseItem(env: Awaited<ReturnType<typeof catbusRun>>) {
  const data = env.data as Record<string, unknown> | undefined
  const item =
    (data?.item as Record<string, unknown> | undefined) ||
    (data?.note as Record<string, unknown> | undefined) ||
    data
  return item && typeof item === 'object' ? item : null
}

async function addNote() {
  const url = noteUrl.value.trim()
  if (!url) {
    ElMessage.warning('请粘贴笔记链接')
    return
  }
  loading.value = true
  try {
    const noteId = extractNoteId(url)
    let meta: Parameters<typeof competitorAdd>[0] = {
      noteUrl: url,
      noteId,
      intervalMinutes: scanInterval.value,
    }
    const noteRef = noteRefForCatbus(url, noteId)
    if (noteRef) {
      try {
        const env = await catbusRun(['xhs', 'item', 'get', noteRef], { timeoutSecs: 90 })
        if (await reportIfCatbusFailed(env, '获取笔记失败')) {
          /* 人机验证等错误已提示；仍允许仅按链接入库 */
        } else {
          const item = parseItem(env)
          if (item) {
            const stats = (item.stats as Record<string, number>) || {}
            const itemUrl = String(item.url || '')
            meta = {
              ...meta,
              // 回写带 xsec_token 的 url，供后续刷新使用
              noteUrl: itemUrl && /xsec_token=/i.test(itemUrl) ? itemUrl : url,
              noteId: String(item.id || noteId),
              title: String(item.title || ''),
              authorName: String(
                (item.author as { name?: string })?.name || item.authorName || '',
              ),
              coverUrl: String(item.coverUrl || item.cover || ''),
              likes: Number(stats.likes ?? item.likes ?? 0),
              collects: Number(stats.collects ?? item.collects ?? 0),
              comments: Number(stats.comments ?? item.comments ?? 0),
              shares: Number(stats.shares ?? item.shares ?? 0),
            }
          }
        }
      } catch (e) {
        await reportCatbusError(e, '获取笔记失败')
      }
    }
    await competitorAdd(meta)
    noteUrl.value = ''
    noteAddOpen.value = false
    ElMessage.success('已加入监控')
    await loadCompetitors()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
  }
}

async function addUser() {
  const url = profileUrl.value.trim()
  if (!url) {
    ElMessage.warning('请粘贴用户主页链接')
    return
  }
  loading.value = true
  try {
    const uid = extractUserId(url)
    let meta: Parameters<typeof watchedUserAdd>[0] = { profileUrl: url, externalUserId: uid }
    if (uid) {
      try {
        const env = await catbusRun(['xhs', 'user', 'get', uid], { timeoutSecs: 90 })
        if (await reportIfCatbusFailed(env, '获取用户失败')) {
          /* 已提示验证链接 */
        } else {
          const data = env.data as Record<string, unknown> | undefined
          const user =
            (data?.user as Record<string, unknown> | undefined) ||
            (data?.profile as Record<string, unknown> | undefined) ||
            data
          if (user) {
            meta = {
              ...meta,
              nickname: String(user.nickname || user.name || ''),
              avatarUrl: String(user.avatar || user.avatarUrl || ''),
              externalUserId: String(user.id || user.userId || uid),
            }
          }
        }
      } catch (e) {
        await reportCatbusError(e, '获取用户失败')
      }
    }
    const row = await watchedUserAdd(meta)
    profileUrl.value = ''
    userAddOpen.value = false
    selectedUserId.value = row.id
    ElMessage.success('已添加用户监控')
    await loadWatchedUsers()
    await syncUser(row)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
  }
}

function markRefreshing(id: string, on: boolean) {
  const next = new Set(refreshingIds.value)
  if (on) next.add(id)
  else next.delete(id)
  refreshingIds.value = next
}

async function onRefreshCompetitor(row: CompetitorRow) {
  markRefreshing(row.id, true)
  try {
    const noteRef = noteRefForCatbus(row.noteUrl, row.noteId)
    if (noteRef && !/xsec_token=/i.test(noteRef) && /^[0-9a-f]{24}$/i.test(noteRef)) {
      ElMessage.warning('当前仅有笔记 ID，缺少 xsec_token，可能获取失败；建议重新添加带 token 的链接')
    }
    const res = await syncCompetitorNote(row, { reportError: true, trigger: '手动刷新' })
    if (!res.ok) return
    ElMessage.success('已刷新')
    await loadCompetitors()
    if (current.value?.id === row.id) {
      const updated = competitors.value.find((c) => c.id === row.id)
      if (updated) await openDetail(updated, detailTab.value)
    }
  } finally {
    markRefreshing(row.id, false)
  }
}

async function removeCompetitor(row: CompetitorRow) {
  try {
    await ElMessageBox.confirm(`移除监控「${row.title || row.noteUrl}」？`, '确认', {
      type: 'warning',
    })
  } catch {
    return
  }
  await competitorDelete(row.id)
  await loadCompetitors()
}

async function openDetail(row: CompetitorRow, tab: string) {
  current.value = row
  detailTab.value = tab
  detailOpen.value = true
  snaps.value = await competitorSnapshots(row.id, 15)
  try {
    analysis.value = row.analysisJson
      ? (JSON.parse(row.analysisJson) as ReturnType<typeof computeAnalysis>)
      : computeAnalysis(row, snaps.value)
  } catch {
    analysis.value = computeAnalysis(row, snaps.value)
  }
  try {
    const chat = JSON.parse(row.chatJson || '{"messages":[]}') as {
      messages?: { role: string; content: string }[]
    }
    chatMessages.value = chat.messages || []
  } catch {
    chatMessages.value = []
  }
  try {
    noteKeywords.value = JSON.parse(row.keywordsJson || '[]') as string[]
  } catch {
    noteKeywords.value = []
  }
  opportunities.value = await competitorListOpportunities(row.id)
  await nextTick()
  if (tab === 'trend') renderChart()
}

function renderChart() {
  if (!chartEl.value) return
  if (!chart) chart = echarts.init(chartEl.value)
  const n = snaps.value.length
  // 点多时缩短标签，并让 axis 自动跳过重叠
  const labels = snaps.value.map((s) => {
    const t = formatDateTime(s.capturedAt)
    return n > 12 ? t.slice(5, 10) : t.slice(5, 16)
  })
  chart.setOption(
    {
      tooltip: { trigger: 'axis' },
      legend: {
        data: ['赞', '藏', '评', '转', '互动'],
        top: 0,
        left: 'center',
      },
      grid: { left: 48, right: 20, top: 40, bottom: n > 8 ? 56 : 40 },
      xAxis: {
        type: 'category',
        data: labels,
        boundaryGap: false,
        axisLabel: {
          hideOverlap: true,
          rotate: n > 8 ? 35 : 0,
          fontSize: 11,
          margin: 10,
          interval: n > 20 ? 'auto' : 0,
        },
      },
      yAxis: { type: 'value', scale: true },
      series: [
        { name: '赞', type: 'line', showSymbol: n <= 20, data: snaps.value.map((s) => s.likes) },
        { name: '藏', type: 'line', showSymbol: n <= 20, data: snaps.value.map((s) => s.collects) },
        { name: '评', type: 'line', showSymbol: n <= 20, data: snaps.value.map((s) => s.comments) },
        { name: '转', type: 'line', showSymbol: n <= 20, data: snaps.value.map((s) => s.shares) },
        { name: '互动', type: 'line', showSymbol: n <= 20, data: snaps.value.map((s) => s.engagement) },
      ],
    },
    true,
  )
  chart.resize()
}

watch(detailTab, async (t) => {
  if (t === 'trend') {
    await nextTick()
    renderChart()
  }
})

async function sendChat() {
  if (!current.value || !chatInput.value.trim()) return
  const message = chatInput.value.trim()
  chatBusy.value = true
  try {
    const reply = await aiCompetitorChat({
      message,
      context: {
        note: current.value,
        analysis: analysis.value,
        snapshots: snaps.value.slice(-30),
        opportunities: opportunities.value.slice(0, 20),
      },
      history: chatMessages.value,
    })
    chatMessages.value = [
      ...chatMessages.value,
      { role: 'user', content: message },
      { role: 'assistant', content: reply },
    ].slice(-40)
    chatInput.value = ''
    await competitorSetChat(current.value.id, JSON.stringify({ messages: chatMessages.value }))
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    chatBusy.value = false
  }
}

async function openComments(row: CompetitorRow) {
  comments.value = await competitorComments(row.id)
  commentsOpen.value = true
}

async function openLogs(row: CompetitorRow) {
  logsPage.value = 1
  logs.value = await competitorRefreshLogs(row.id)
  logsOpen.value = true
}

async function scanOpp() {
  if (!current.value) return
  try {
    await competitorSetKeywords(current.value.id, noteKeywords.value)
    const r = await competitorScanOpportunities(current.value.id)
    ElMessage.success(`扫描完成，匹配 ${r.created} 条`)
    opportunities.value = await competitorListOpportunities(current.value.id)
    await loadCompetitors()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function removeOpp(id: string) {
  await competitorDeleteOpportunity(id)
  if (current.value) {
    opportunities.value = await competitorListOpportunities(current.value.id)
    await loadCompetitors()
  }
}

async function saveInterval() {
  try {
    await competitorPutSettings({ scanIntervalMinutes: scanInterval.value })
    ElMessage.success('已保存扫描间隔')
    intervalOpen.value = false
    await loadCompetitors()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

function addGlobalKw() {
  const t = kwInput.value.trim()
  if (!t) return
  if (!globalKeywords.value.includes(t)) globalKeywords.value = [...globalKeywords.value, t]
  kwInput.value = ''
}

async function saveGlobalKw() {
  try {
    await competitorPutSettings({ keywords: globalKeywords.value })
    ElMessage.success('已保存全局关键词')
    globalKwOpen.value = false
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function syncUser(row: WatchedUserRow) {
  markRefreshing(row.id, true)
  try {
    const uid = row.externalUserId || extractUserId(row.profileUrl)
    if (!uid) throw new Error('无法解析用户 ID')
    const env = await catbusRun(['xhs', 'user', 'get', uid], { timeoutSecs: 90 })
    if (await reportIfCatbusFailed(env, '同步用户失败')) return
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
    })
    ElMessage.success(newEvents.length ? `同步完成，新动态 ${newEvents.length}` : '同步完成')
    await loadWatchedUsers()
    if (selectedUserId.value === row.id) await loadEvents(row.id)
  } catch (e) {
    await reportCatbusError(e, '同步用户失败')
  } finally {
    markRefreshing(row.id, false)
  }
}

async function removeUser(row: WatchedUserRow) {
  try {
    await ElMessageBox.confirm(`移除用户「${row.nickname || row.profileUrl}」？`, '确认', {
      type: 'warning',
    })
  } catch {
    return
  }
  await watchedUserDelete(row.id)
  if (selectedUserId.value === row.id) selectedUserId.value = ''
  await loadWatchedUsers()
}

async function markAllRead() {
  if (!selectedUserId.value) return
  await watchedUserMarkRead(selectedUserId.value)
  await loadEvents(selectedUserId.value)
  await loadWatchedUsers()
}

watch(viewTab, (tab) => {
  if (tab === 'users' && !watchedUsers.value.length) loadWatchedUsers()
})

watch(selectedUserId, (id) => {
  if (id) loadEvents(id)
})

onMounted(async () => {
  await loadSettings()
  await loadCompetitors()
})

onUnmounted(() => {
  chart?.dispose()
  chart = null
})
</script>

<template>
  <div class="mf-ops__comp-page" v-loading="loading">
    <section class="mf-ops__panel mf-ops__discover-results">
      <div class="mf-ops__panel-head mf-ops__panel-head--comp-tabs">
        <div class="mf-ops__comp-tabs" role="tablist">
          <button
            type="button"
            class="mf-ops__seg-btn"
            :class="{ 'is-active': viewTab === 'notes' }"
            @click="viewTab = 'notes'"
          >
            笔记监控
          </button>
          <button
            type="button"
            class="mf-ops__seg-btn"
            :class="{ 'is-active': viewTab === 'users' }"
            @click="viewTab = 'users'"
          >
            用户监控
          </button>
        </div>
        <div class="mf-ops__comp-head-actions">
          <el-select v-if="viewTab === 'notes'" v-model="sortBy" size="small" style="width: 110px">
            <el-option value="time" label="按时间" />
            <el-option value="score" label="按得分" />
            <el-option value="likes" label="按点赞" />
          </el-select>
          <el-tag size="small" type="info">
            {{ viewTab === 'users' ? watchedUsers.length : competitors.length }} 个
          </el-tag>
          <el-button
            size="small"
            @click="viewTab === 'users' ? loadWatchedUsers() : loadCompetitors()"
          >
            刷新
          </el-button>
          <el-button v-if="viewTab === 'notes'" size="small" @click="intervalOpen = true">
            扫描间隔
          </el-button>
          <el-button
            size="small"
            :type="monitorRunning ? 'danger' : 'success'"
            plain
            @click="toggleMonitor()"
          >
            {{ monitorRunning ? '停止监控' : '启动监控' }}
          </el-button>
          <el-button v-if="viewTab === 'notes'" size="small" @click="globalKwOpen = true">
            全局关键词
          </el-button>
          <el-button
            size="small"
            type="primary"
            @click="viewTab === 'users' ? (userAddOpen = true) : (noteAddOpen = true)"
          >
            {{ viewTab === 'users' ? '添加用户' : '添加竞品' }}
          </el-button>
        </div>
      </div>

      <div class="mf-ops__panel-body">
        <!-- 笔记监控 -->
        <template v-if="viewTab === 'notes'">
          <p class="mf-ops__comp-hint">
            笔记监控上限 <strong>{{ MONITOR_LIMIT }}</strong> 篇，当前
            {{ competitors.length }} / {{ MONITOR_LIMIT }}。刷新需有效 PC 搜索账号。
            <template v-if="monitorRunning">
              · <strong style="color: var(--el-color-success)">后台监控运行中</strong>（切页不停）
            </template>
          </p>
          <div v-if="sortedCompetitors.length" class="mf-ops__comp-list">
            <article v-for="item in sortedCompetitors" :key="item.id" class="mf-ops__comp-card">
              <div class="mf-ops__comp-card-main">
                <div class="mf-ops__comp-cover">
                  <img
                    v-if="coverSrc(item)"
                    :src="coverSrc(item)"
                    alt=""
                    referrerpolicy="no-referrer"
                  />
                  <div v-else class="mf-ops__comp-cover-empty">无封面</div>
                </div>
                <div class="mf-ops__comp-body">
                  <div class="mf-ops__comp-title-row">
                    <strong>{{ item.title || '未拉取标题' }}</strong>
                    <el-tag
                      v-if="item.analysisScore > 0"
                      size="small"
                      type="primary"
                      effect="plain"
                    >
                      {{ item.analysisGrade || '—' }} {{ item.analysisScore }}
                    </el-tag>
                  </div>
                  <p class="mf-ops__comp-meta">
                    <em class="mf-ops__comp-author">{{ item.authorName || '未知作者' }}</em>
                    · 间隔 {{ item.intervalMinutes || 60 }} 分钟 (±{{ INTERVAL_JITTER }})
                    <template v-if="item.lastRefreshedAt">
                      · 最近 {{ formatDateTime(item.lastRefreshedAt) }}
                    </template>
                    <template v-if="item.nextRunAt">
                      · 下次约 {{ formatDateTime(item.nextRunAt) }}
                    </template>
                  </p>
                  <button
                    v-if="item.noteUrl"
                    type="button"
                    class="mf-ops__comp-url"
                    @click="openLink(item.noteUrl)"
                  >
                    {{ item.noteUrl }}
                  </button>
                  <div class="mf-ops__comp-metrics">
                    <div><span>赞</span><b>{{ fmt(item.likes) }}</b></div>
                    <div><span>藏</span><b>{{ fmt(item.collects) }}</b></div>
                    <div><span>评</span><b>{{ fmt(item.comments) }}</b></div>
                    <div><span>转</span><b>{{ fmt(item.shares) }}</b></div>
                  </div>
                  <p v-if="item.lastError" class="mf-ops__comp-error">{{ item.lastError }}</p>
                  <div class="mf-ops__comp-actions">
                    <div class="mf-ops__comp-actions-main">
                      <el-button
                        size="small"
                        type="primary"
                        plain
                        :icon="Refresh"
                        :loading="refreshingIds.has(item.id)"
                        @click="onRefreshCompetitor(item)"
                      >
                        刷新
                      </el-button>
                      <span class="mf-ops__comp-actions-sep" />
                      <el-button
                        size="small"
                        plain
                        :icon="TrendCharts"
                        @click="openDetail(item, 'trend')"
                      >
                        趋势
                      </el-button>
                      <el-button
                        size="small"
                        type="success"
                        plain
                        :icon="DataAnalysis"
                        @click="openDetail(item, 'analysis')"
                      >
                        分析
                      </el-button>
                      <el-button
                        size="small"
                        plain
                        :icon="ChatDotRound"
                        @click="openDetail(item, 'assistant')"
                      >
                        助手
                      </el-button>
                      <el-button size="small" plain :icon="Comment" @click="openComments(item)">
                        评论
                      </el-button>
                      <el-badge
                        class="mf-ops__comp-opp-badge"
                        type="danger"
                        :value="item.opportunityCount || 0"
                        :hidden="!(item.opportunityCount > 0)"
                        :max="99"
                      >
                        <el-button
                          size="small"
                          type="warning"
                          plain
                          :icon="Opportunity"
                          @click="openDetail(item, 'opportunity')"
                        >
                          商机
                        </el-button>
                      </el-badge>
                    </div>
                    <div class="mf-ops__comp-actions-end">
                      <el-button
                        size="small"
                        plain
                        type="primary"
                        :icon="Clock"
                        title="执行记录"
                        @click="openLogs(item)"
                      />
                      <el-button
                        size="small"
                        plain
                        type="danger"
                        :icon="Delete"
                        @click="removeCompetitor(item)"
                      />
                    </div>
                  </div>
                </div>
              </div>
            </article>
          </div>
          <div v-else class="mf-ops__empty mf-ops__empty--compact">
            <strong>暂无监控笔记</strong>
            <p>点击「添加竞品」粘贴笔记链接，或在发现/内容库点「监控」。</p>
          </div>
        </template>

        <!-- 用户监控 -->
        <template v-else>
          <p class="mf-ops__comp-hint">监控竞品账号新发笔记与互动变化。同步需有效 PC 搜索账号。</p>
          <div class="mf-ops__wu-layout">
            <div class="mf-ops__comp-list">
              <article
                v-for="u in watchedUsers"
                :key="u.id"
                class="mf-ops__comp-card mf-ops__wu-card"
                :class="{ 'is-selected': u.id === selectedUserId }"
                @click="selectedUserId = u.id"
              >
                <div class="mf-ops__comp-card-main">
                  <div class="mf-ops__wu-avatar">
                    <img
                      v-if="u.avatarUrl"
                      :src="u.avatarUrl"
                      alt=""
                      referrerpolicy="no-referrer"
                    />
                  </div>
                  <div class="mf-ops__comp-body">
                    <div class="mf-ops__comp-title-row">
                      <strong>{{ u.nickname || u.externalUserId || '未命名用户' }}</strong>
                      <el-tag v-if="u.unreadEvents" size="small" type="danger">
                        {{ u.unreadEvents }} 未读
                      </el-tag>
                    </div>
                    <p class="mf-ops__comp-meta">
                      笔记约 {{ u.noteCount }} ·
                      <template v-if="u.lastSyncedAt">
                        最近同步 {{ formatDateTime(u.lastSyncedAt) }}
                      </template>
                      <template v-else>尚未同步</template>
                    </p>
                    <button
                      v-if="u.profileUrl"
                      type="button"
                      class="mf-ops__comp-url"
                      @click.stop="openLink(u.profileUrl)"
                    >
                      {{ u.profileUrl }}
                    </button>
                    <p v-if="u.lastError" class="mf-ops__wu-alert-line">{{ u.lastError }}</p>
                    <div class="mf-ops__comp-actions" @click.stop>
                      <div class="mf-ops__comp-actions-main">
                        <el-button
                          size="small"
                          type="primary"
                          plain
                          :loading="refreshingIds.has(u.id)"
                          @click="syncUser(u)"
                        >
                          同步
                        </el-button>
                      </div>
                      <div class="mf-ops__comp-actions-end">
                        <el-button
                          size="small"
                          plain
                          type="danger"
                          :icon="Delete"
                          @click="removeUser(u)"
                        />
                      </div>
                    </div>
                  </div>
                </div>
              </article>
              <div v-if="!watchedUsers.length" class="mf-ops__empty mf-ops__empty--compact">
                <strong>暂无监控用户</strong>
                <p>点击「添加用户」粘贴小红书主页链接。</p>
              </div>
            </div>

            <aside class="mf-ops__wu-events">
              <div class="mf-ops__wu-events-head">
                <strong>{{ selectedUser?.nickname || '动态' }}</strong>
                <el-button
                  size="small"
                  :disabled="!selectedUserId"
                  @click="markAllRead"
                >
                  全部已读
                </el-button>
              </div>
              <div class="mf-ops__wu-events-list">
                <div
                  v-for="ev in events"
                  :key="ev.id"
                  class="mf-ops__wu-event"
                  :class="{ 'is-unread': !ev.isRead }"
                >
                  <strong>{{ ev.title || ev.eventType }}</strong>
                  <p>{{ formatDateTime(ev.createdAt) }}</p>
                  <button
                    v-if="ev.noteUrl"
                    type="button"
                    class="mf-ops__comp-url"
                    @click="openLink(ev.noteUrl)"
                  >
                    打开笔记
                  </button>
                </div>
                <p v-if="!events.length" class="mf-ops__comp-hint">暂无动态，点「同步」拉取。</p>
              </div>
            </aside>
          </div>
        </template>
      </div>
    </section>

    <!-- 添加竞品 -->
    <el-dialog v-model="noteAddOpen" title="添加竞品笔记" width="480px">
      <el-input
        v-model="noteUrl"
        placeholder="粘贴带 xsec_token 的笔记链接（explore/…?xsec_token=…）"
      />
      <template #footer>
        <el-button size="small" @click="noteAddOpen = false">取消</el-button>
        <el-button size="small" type="primary" @click="addNote">添加</el-button>
      </template>
    </el-dialog>

    <!-- 添加用户 -->
    <el-dialog v-model="userAddOpen" title="添加监控用户" width="480px">
      <el-input v-model="profileUrl" placeholder="粘贴用户主页 URL" />
      <template #footer>
        <el-button size="small" @click="userAddOpen = false">取消</el-button>
        <el-button size="small" type="primary" @click="addUser">添加</el-button>
      </template>
    </el-dialog>

    <!-- 扫描间隔 -->
    <el-dialog v-model="intervalOpen" title="商机扫描间隔" width="400px">
      <el-form label-width="120px" size="small">
        <el-form-item label="间隔（分钟）">
          <el-input-number v-model="scanInterval" :min="30" :max="1440" />
        </el-form-item>
      </el-form>
      <p class="mf-ops__comp-hint">
        最小 30 分钟。配置后点「启动监控」才会后台刷新；到期笔记会错开拟人间隔逐条执行（每轮最多 1
        条），不会同时齐刷。切页不停。
      </p>
      <template #footer>
        <el-button size="small" @click="intervalOpen = false">取消</el-button>
        <el-button size="small" type="primary" @click="saveInterval">保存</el-button>
      </template>
    </el-dialog>

    <!-- 全局关键词 -->
    <el-dialog v-model="globalKwOpen" title="全局商机关键词" width="480px">
      <div style="display: flex; gap: 8px; margin-bottom: 10px">
        <el-input v-model="kwInput" size="small" placeholder="输入关键词回车添加" @keydown.enter.prevent="addGlobalKw" />
        <el-button size="small" @click="addGlobalKw">添加</el-button>
      </div>
      <div style="display: flex; flex-wrap: wrap; gap: 6px">
        <el-tag
          v-for="k in globalKeywords"
          :key="k"
          closable
          @close="globalKeywords = globalKeywords.filter((x) => x !== k)"
        >
          {{ k }}
        </el-tag>
      </div>
      <template #footer>
        <el-button size="small" @click="globalKwOpen = false">取消</el-button>
        <el-button size="small" type="primary" @click="saveGlobalKw">保存</el-button>
      </template>
    </el-dialog>

    <!-- 详情 -->
    <el-dialog
      v-model="detailOpen"
      :title="current?.title || '竞品详情'"
      width="640px"
      destroy-on-close
    >
      <el-tabs v-model="detailTab">
        <el-tab-pane label="趋势" name="trend">
          <div ref="chartEl" style="width: 100%; height: 320px" />
        </el-tab-pane>
        <el-tab-pane label="分析" name="analysis">
          <template v-if="analysis">
            <p>
              <el-tag type="success">{{ analysis.grade }}</el-tag>
              评分 {{ analysis.score }} · 增速 {{ analysis.growthPct }}%
            </p>
            <p>{{ analysis.summary }}</p>
            <ul>
              <li v-for="s in analysis.suggestions" :key="s">{{ s }}</li>
            </ul>
          </template>
        </el-tab-pane>
        <el-tab-pane label="助手" name="assistant">
          <div style="max-height: 280px; overflow: auto; margin-bottom: 10px">
            <div v-for="(m, i) in chatMessages" :key="i" style="margin-bottom: 10px">
              <strong>{{ m.role === 'assistant' ? '助手' : '我' }}</strong>
              <p style="margin: 4px 0 0; white-space: pre-wrap; font-size: 13px">{{ m.content }}</p>
            </div>
          </div>
          <el-input v-model="chatInput" type="textarea" :rows="2" placeholder="基于监控数据提问…" />
          <el-button
            style="margin-top: 8px"
            type="primary"
            size="small"
            :loading="chatBusy"
            @click="sendChat"
          >
            发送
          </el-button>
        </el-tab-pane>
        <el-tab-pane label="商机" name="opportunity">
          <div style="display: flex; gap: 8px; margin-bottom: 10px; flex-wrap: wrap">
            <el-input
              v-model="kwInput"
              size="small"
              placeholder="本帖额外关键词"
              style="width: 180px"
              @keydown.enter.prevent="
                () => {
                  const t = kwInput.trim()
                  if (t && !noteKeywords.includes(t)) noteKeywords = [...noteKeywords, t]
                  kwInput = ''
                }
              "
            />
            <el-button size="small" type="primary" @click="scanOpp">扫描评论商机</el-button>
          </div>
          <div style="display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 12px">
            <el-tag
              v-for="k in noteKeywords"
              :key="k"
              closable
              size="small"
              @close="noteKeywords = noteKeywords.filter((x) => x !== k)"
            >
              {{ k }}
            </el-tag>
          </div>
          <div v-for="o in opportunities" :key="o.id" class="mf-ops__wu-event" style="margin-bottom: 8px">
            <strong>#{{ o.keyword }} · {{ o.authorNickname || '用户' }}</strong>
            <p>{{ o.commentText }}</p>
            <el-button size="small" link type="danger" @click="removeOpp(o.id)">删除</el-button>
          </div>
          <p v-if="!opportunities.length" class="mf-ops__comp-hint">
            先刷新拉取评论，再配置关键词后扫描。
          </p>
        </el-tab-pane>
      </el-tabs>
    </el-dialog>

    <el-dialog v-model="commentsOpen" title="已存评论" width="520px">
      <div v-for="c in comments" :key="c.id" class="mf-ops__wu-event" style="margin-bottom: 8px">
        <strong>{{ c.userName || '用户' }} · 赞 {{ c.likeCount }}</strong>
        <p>{{ c.content }}</p>
      </div>
      <p v-if="!comments.length" class="mf-ops__comp-hint">暂无评论，请先刷新笔记。</p>
    </el-dialog>

    <el-dialog v-model="logsOpen" title="执行记录" width="640px" class="mf-ops__logs-dialog">
      <el-table
        v-if="logs.length"
        :data="pagedLogs"
        stripe
        size="small"
        empty-text="暂无记录"
        style="width: 100%"
      >
        <el-table-column label="时间" min-width="160">
          <template #default="{ row }">{{ formatDateTime(row.createdAt) }}</template>
        </el-table-column>
        <el-table-column label="状态" width="90" align="center">
          <template #default="{ row }">
            <el-tag size="small" :type="logStatusType(row.status)" effect="plain">
              {{ formatLogStatus(row.status) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="触发方式" min-width="140">
          <template #default="{ row }">{{ formatLogMessage(row.message) }}</template>
        </el-table-column>
      </el-table>
      <p v-else class="mf-ops__comp-hint">暂无记录。</p>
      <div v-if="logs.length" class="mf-ops__logs-pager">
        <el-pagination
          v-model:current-page="logsPage"
          background
          layout="total, prev, pager, next"
          :page-size="LOG_PAGE_SIZE"
          :total="logs.length"
          small
        />
      </div>
    </el-dialog>
  </div>
</template>

<style scoped lang="scss">
.mf-ops__logs-pager {
  display: flex;
  justify-content: flex-end;
  margin-top: 14px;
}
</style>
