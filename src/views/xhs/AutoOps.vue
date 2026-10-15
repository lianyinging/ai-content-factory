<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  browserCdpStatusAll,
  listBrowserProfiles,
  openBrowserUrl,
  parseProfileSession,
  probeXhsCommentMatch,
  probeXhsCustomReply,
  probeXhsFilterPanel,
  type BrowserProfile,
  type BrowserStatus,
} from '@/api/cdp'
import {
  acqReplyHistoryClear,
  acqReplyHistoryList,
  autoOpsGetBinding,
  kbListBases,
  type AcqReplyHistoryRow,
  type KbBase,
} from '@/api/knowledge'
import AutoOpsChatPanel from '@/components/AutoOpsChatPanel.vue'
import { useXhsAutoOpsStore } from '@/state/xhsAutoOps'

const store = useXhsAutoOpsStore()
const {
  profileId,
  enabled,
  systemPrompt,
  selectedKbIds,
  intervalSec,
  polling,
  logs,
  lastBadge,
  csRunStats,
  acqRunning,
  acqLogs,
  acqLastResult,
  pushAcqLog,
  pushAcqStepLog,
  startAcq: startAcqJob,
  stopAcqHint,
  resetAcqStuck,
} = store

const activeTab = ref<'cs' | 'acq'>('cs')
const chatPanelOpen = ref(false)
const csConfigOpen = ref(false)
const acqConfigOpen = ref(false)
const acqHistoryOpen = ref(false)
const acqHistoryLoading = ref(false)
const acqHistoryRows = ref<AcqReplyHistoryRow[]>([])

const profiles = ref<BrowserProfile[]>([])
const statuses = ref<BrowserStatus[]>([])
const bases = ref<KbBase[]>([])
const busy = ref(false)
const csLogEl = ref<HTMLElement | null>(null)

/** AI 获客本地配置 */
const ACQ_CFG_KEY = 'xhs-acq-config'
const acqSearchKeyword = ref('')
const acqCommentKeyword = ref('')
const acqSort = ref('最新')
const acqReply = ref('你好，看到你的评论了～方便的话可以私信我详细聊聊～')
const acqMaxNotes = ref(5)
const acqMaxReplies = ref(1)
/** 单篇笔记内 .note-content 下滑加载更多评论次数 */
const acqMaxScrolls = ref(5)
const acqLogEl = ref<HTMLElement | null>(null)
const acqSortOptions = ref<string[]>(['综合', '最新', '最多点赞', '最多评论', '最多收藏'])
/** 探测链路已验证完毕，默认隐藏；需要时改为 true */
const showAcqProbes = false

function loadAcqConfig() {
  try {
    const raw = localStorage.getItem(ACQ_CFG_KEY)
    if (!raw) return
    const c = JSON.parse(raw) as {
      profileId?: string
      searchKeyword?: string
      commentKeyword?: string
      sort?: string
      reply?: string
      maxNotes?: number
      maxReplies?: number
      maxScrolls?: number
    }
    if (c.profileId) profileId.value = c.profileId
    if (typeof c.searchKeyword === 'string') acqSearchKeyword.value = c.searchKeyword
    if (typeof c.commentKeyword === 'string') acqCommentKeyword.value = c.commentKeyword
    if (typeof c.sort === 'string' && c.sort) acqSort.value = c.sort
    if (typeof c.reply === 'string') acqReply.value = c.reply
    if (typeof c.maxNotes === 'number') acqMaxNotes.value = c.maxNotes
    if (typeof c.maxReplies === 'number') acqMaxReplies.value = c.maxReplies
    if (typeof c.maxScrolls === 'number') acqMaxScrolls.value = c.maxScrolls
  } catch {
    /* ignore */
  }
}

function saveAcqConfig() {
  localStorage.setItem(
    ACQ_CFG_KEY,
    JSON.stringify({
      profileId: profileId.value,
      searchKeyword: acqSearchKeyword.value,
      commentKeyword: acqCommentKeyword.value,
      sort: acqSort.value,
      reply: acqReply.value,
      maxNotes: acqMaxNotes.value,
      maxReplies: acqMaxReplies.value,
      maxScrolls: acqMaxScrolls.value,
    }),
  )
  ElMessage.success('已保存获客配置')
}

const online = computed(() => statuses.value.find((s) => s.profileId === profileId.value)?.cdpOnline)

const csProfileLabel = computed(() => {
  const p = profiles.value.find((x) => x.id === profileId.value)
  return p?.name || profileId.value || '—'
})

/** 客服 · 云端风控机位卡 */
const csCloudCard = computed(() => {
  if (!online.value) {
    return {
      tone: 'off' as const,
      title: 'CDP 离线',
      sub: '等待 CDP 上线',
      pill: '机位未上线',
      batt: '--',
      battOn: false,
    }
  }
  if (!enabled.value) {
    return {
      tone: 'off' as const,
      title: '客服未启用',
      sub: '请在配置中打开启用开关',
      pill: '服务未启用',
      batt: '100%',
      battOn: true,
    }
  }
  if (polling.value) {
    return {
      tone: 'run' as const,
      title: '自动回复中',
      sub: '轮询未读 · 知识库+LLM',
      pill: '云端值守中',
      batt: '100%',
      battOn: true,
    }
  }
  return {
    tone: 'ready' as const,
    title: '客服待命',
    sub: '已接管浏览器会话',
    pill: '手机状态已接管',
    batt: '100%',
    battOn: true,
  }
})

const csResultBadge = computed(() => {
  if (!online.value) return { text: '离线', tone: 'idle' as const }
  if (!enabled.value) return { text: '未启用', tone: 'warn' as const }
  if (polling.value) return { text: '值守中', tone: 'run' as const }
  if (csRunStats.value.repliesSent > 0 || csRunStats.value.sessionsTouched > 0) {
    return { text: '已值守', tone: 'ok' as const }
  }
  return { text: '待命', tone: 'idle' as const }
})

const csResultSummary = computed(() => {
  const s = (csRunStats.value.lastSummary || '').trim()
  if (s) return s.length <= 48 ? s : `${s.slice(0, 48)}…`
  if (polling.value) return `间隔 ${intervalSec.value}s · 知识库 ${selectedKbIds.value.length} 个`
  return '点击「开始自动回复」后，此处展示本轮角标、回复与会话触及'
})

const csResultFoot = computed(() => {
  if (!online.value) return 'CDP 离线'
  if (!enabled.value) return '客服未启用'
  if (polling.value) return '自动回复中'
  return '客服待命'
})

/** 获客 · 云端风控机位卡文案 */
const acqCloudCard = computed(() => {
  if (acqRunning.value) {
    return {
      tone: 'run' as const,
      title: '获客执行中',
      sub: '云端执行链路中',
      pill: '云端执行中',
      batt: '100%',
      battOn: true,
    }
  }
  if (online.value) {
    return {
      tone: 'ready' as const,
      title: '获客待命',
      sub: '已接管浏览器会话',
      pill: '手机状态已接管',
      batt: '100%',
      battOn: true,
    }
  }
  return {
    tone: 'off' as const,
    title: 'CDP 离线',
    sub: '等待 CDP 上线',
    pill: '机位未上线',
    batt: '--',
    battOn: false,
  }
})

const acqResultBadge = computed(() => {
  if (acqRunning.value) return { text: '执行中', tone: 'run' as const }
  const r = acqLastResult.value
  if (!r) return { text: '待命', tone: 'idle' as const }
  if (r.ok) return { text: '已完成', tone: 'ok' as const }
  if (r.message === '执行中…') return { text: '执行中', tone: 'run' as const }
  return { text: '未完成', tone: 'warn' as const }
})

const acqResultSummary = computed(() => {
  const r = acqLastResult.value
  if (!r) return '点击「开始获客」后，此处展示本轮打开笔记、匹配评论与发送数'
  const msg = (r.message || '').trim()
  if (msg.length <= 48) return msg
  return `${msg.slice(0, 48)}…`
})

const acqResultFoot = computed(() => {
  if (acqRunning.value) return '获客执行中'
  if (online.value) return '获客待命'
  return 'CDP 离线'
})

function formatAcqHistoryTime(ts: number) {
  if (!ts) return '—'
  const d = new Date(ts * 1000)
  if (Number.isNaN(d.getTime())) return '—'
  return d.toLocaleString('zh-CN', { hour12: false })
}

async function openAcqHistory() {
  acqHistoryOpen.value = true
  await loadAcqHistory()
}

async function loadAcqHistory() {
  const pid = (profileId.value || '').trim()
  if (!pid) {
    acqHistoryRows.value = []
    ElMessage.warning('请先选择 Profile')
    return
  }
  acqHistoryLoading.value = true
  try {
    acqHistoryRows.value = await acqReplyHistoryList(pid, 200)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    acqHistoryLoading.value = false
  }
}

async function clearAcqHistory() {
  const pid = (profileId.value || '').trim()
  if (!pid) return
  try {
    await ElMessageBox.confirm('清空当前 Profile 下全部获客评论回复记录？', '确认', {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    await acqReplyHistoryClear(pid)
    acqHistoryRows.value = []
    ElMessage.success('已清空历史')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

watch(
  () => logs.value.length,
  async () => {
    await nextTick()
    const el = csLogEl.value
    if (el) el.scrollTop = el.scrollHeight
  },
)

watch(
  () => acqLogs.value?.length ?? 0,
  async () => {
    await nextTick()
    const el = acqLogEl.value
    if (el) el.scrollTop = el.scrollHeight
  },
)

watch(
  online,
  (v) => {
    store.setCdpOnline(!!v)
  },
  { immediate: true },
)

async function refresh() {
  busy.value = true
  try {
    profiles.value = await listBrowserProfiles()
    const sum = await browserCdpStatusAll()
    statuses.value = sum.profiles || []
    bases.value = await kbListBases().catch(() => [])
    if (!profileId.value) {
      const logged = profiles.value.find((p) => parseProfileSession(p.sessionJson).xhs?.loggedIn)
      profileId.value =
        statuses.value.find((s) => s.cdpOnline)?.profileId || logged?.id || profiles.value[0]?.id || ''
    }
    if (profileId.value) {
      await loadBinding()
      await store.loadConsoleLogs(profileId.value)
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busy.value = false
  }
}

async function loadBinding() {
  if (!profileId.value) return
  // 轮询进行中：只刷新 CDP 状态，避免用库里旧配置覆盖内存中的运行参数
  if (polling.value) {
    await store.loadConsoleLogs(profileId.value)
    return
  }
  const b = await autoOpsGetBinding(profileId.value)
  if (!b) {
    store.applyBinding({ enabled: false, selectedKbIds: [] })
  } else {
    let ids: string[] = []
    try {
      ids = JSON.parse(b.kbBaseIdsJson || '[]')
    } catch {
      ids = []
    }
    store.applyBinding({
      enabled: b.enabled,
      systemPrompt: b.systemPrompt || undefined,
      selectedKbIds: ids,
    })
  }
  await store.loadConsoleLogs(profileId.value)
}

async function saveBinding() {
  if (!profileId.value) return
  busy.value = true
  try {
    await store.persistBinding()
    ElMessage.success('已保存绑定')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busy.value = false
  }
}

async function openIm() {
  try {
    await openBrowserUrl('https://www.xiaohongshu.com/notification', profileId.value || undefined)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

function onIntervalChange(v: number | undefined) {
  if (typeof v === 'number') store.setIntervalSec(v)
}

async function probeAcqFilters() {
  if (!profileId.value) {
    ElMessage.warning('请选择 Profile')
    return
  }
  if (!online.value) {
    ElMessage.warning('CDP 离线')
    return
  }
  try {
    pushAcqLog('探测筛选面板…', '启动任务', 'neutral')
    const r = await probeXhsFilterPanel(profileId.value)
    pushAcqLog(r.message || (r.ok ? '探测完成' : '探测失败'), '探测筛选', r.ok ? 'ok' : 'warn')
    const visible = (r.tags || []).filter((t) => t.visible && t.text).map((t) => t.text)
    if (visible.length) {
      const merged = Array.from(new Set([...acqSortOptions.value, ...visible]))
      acqSortOptions.value = merged
      pushAcqLog(`可见排序项: ${visible.join(' / ')}`, '步骤 1/1', 'cyan')
      ElMessage.success(`探测到 ${visible.length} 个可见筛选项`)
    } else {
      ElMessage.warning(r.panelFound ? '面板已出现但无可点击项' : '未出现筛选面板，请先打开搜索结果页')
    }
    for (const s of r.steps || []) {
      pushAcqStepLog(s.ok, s.name, s.detail)
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    pushAcqLog(`探测异常: ${msg}`, '系统异常', 'err')
    ElMessage.error(msg)
  }
}

/** 当前浏览器已打开笔记 → 鼠标定位评论滑动区 → 滚轮 → 按评论关键词匹配 */
async function probeAcqComments() {
  if (!profileId.value) {
    ElMessage.warning('请选择 Profile')
    return
  }
  if (!online.value) {
    ElMessage.warning('CDP 离线')
    return
  }
  const commentKw = acqCommentKeyword.value.trim()
  if (!commentKw) {
    ElMessage.warning('请填写评论匹配关键词')
    return
  }
  try {
    pushAcqLog(
      `探测评论匹配 · 关键词「${commentKw}」· 下滑 ${acqMaxScrolls.value} 次`,
      '启动任务',
      'neutral',
    )
    const r = await probeXhsCommentMatch({
      profileId: profileId.value,
      commentKeyword: commentKw,
      maxScrolls: acqMaxScrolls.value,
    })
    pushAcqLog(r.message || (r.ok ? '探测完成' : '探测失败'), '探测匹配', r.ok ? 'ok' : 'warn')
    if (!r.scrollerFound) {
      ElMessage.warning('未找到评论滑动区，请先在浏览器中打开一篇笔记详情')
    } else if ((r.matched || []).length) {
      ElMessage.success(`命中 ${r.matched.length} 条评论`)
      for (const m of r.matched.slice(0, 20)) {
        pushAcqLog(
          `命中#${m.index}「${m.matchedKeyword}」${m.hasReplyIcon ? '有回复入口' : '无回复入口'}：${m.text}`,
          '匹配评论',
          'purple',
        )
      }
    } else {
      ElMessage.warning(r.commentsFound ? '已扫描评论区，但无匹配' : '未找到评论区')
    }
    for (const s of r.steps || []) {
      pushAcqStepLog(s.ok, s.name, s.detail)
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    pushAcqLog(`探测评论异常: ${msg}`, '系统异常', 'err')
    ElMessage.error(msg)
  }
}

/** 当前已打开笔记：匹配评论 → 点回复 → 填入自定义文案并真正发送 1 条（探测完整链路） */
async function probeAcqCustomReply() {
  if (!profileId.value) {
    ElMessage.warning('请选择 Profile')
    return
  }
  if (!online.value) {
    ElMessage.warning('CDP 离线')
    return
  }
  const commentKw = acqCommentKeyword.value.trim()
  const reply = acqReply.value.trim()
  if (!commentKw) {
    ElMessage.warning('请填写评论匹配关键词')
    return
  }
  if (!reply) {
    ElMessage.warning('请填写自定义回复')
    return
  }
  try {
    pushAcqLog(
      `探测自定义回复 · 评论词「${commentKw}」· 文案「${reply.slice(0, 40)}」· 下滑 ${acqMaxScrolls.value}`,
      '启动任务',
      'neutral',
    )
    const r = await probeXhsCustomReply({
      profileId: profileId.value,
      commentKeyword: commentKw,
      replyText: reply,
      maxScrolls: acqMaxScrolls.value,
      maxReplies: 1,
    })
    pushAcqLog(r.message, '探测回复', r.ok ? 'ok' : 'warn')
    for (const s of r.steps || []) {
      pushAcqStepLog(s.ok, s.name, s.detail)
    }
    if (r.ok) ElMessage.success(r.message)
    else ElMessage.warning(r.message || '探测未发送成功')
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    pushAcqLog(`探测自定义回复异常: ${msg}`, '系统异常', 'err')
    ElMessage.error(msg)
  }
}

function startAcq() {
  store.setCdpOnline(!!online.value)
  void startAcqJob({
    profileId: profileId.value,
    searchKeyword: acqSearchKeyword.value,
    commentKeyword: acqCommentKeyword.value,
    sortLabel: acqSort.value,
    replyText: acqReply.value,
    maxNotes: acqMaxNotes.value,
    maxReplies: acqMaxReplies.value,
    maxScrolls: acqMaxScrolls.value,
  })
}

onMounted(() => {
  loadAcqConfig()
  void refresh()
})
</script>

<template>
  <div class="auto-ops">
    <section class="mf-ops__panel">
      <div class="mf-ops__panel-head">
        <strong>小红书自动运营</strong>
      </div>
      <div class="mf-ops__panel-body">
        <div class="auto-ops__tabbar" role="tablist">
          <button
            type="button"
            role="tab"
            class="auto-ops__tab"
            :class="{ 'is-active': activeTab === 'cs' }"
            :aria-selected="activeTab === 'cs'"
            @click="activeTab = 'cs'"
          >
            <span>AI 客服</span>
            <el-badge v-if="polling" is-dot class="auto-ops__tab-dot" />
          </button>
          <button
            type="button"
            role="tab"
            class="auto-ops__tab"
            :class="{ 'is-active': activeTab === 'acq' }"
            :aria-selected="activeTab === 'acq'"
            @click="activeTab = 'acq'"
          >
            <span>AI 获客</span>
            <el-badge v-if="acqRunning" is-dot class="auto-ops__tab-dot" />
          </button>
        </div>

        <div v-show="activeTab === 'cs'" class="ops-pane" role="tabpanel">
          <div class="cs-actions">
            <el-button type="primary" :disabled="polling || !enabled" @click="store.startPoll">
              开始自动回复
            </el-button>
            <el-button :disabled="!polling" @click="store.stopPoll()">停止</el-button>
            <el-button @click="csConfigOpen = true">配置</el-button>
            <el-button @click="chatPanelOpen = true">会话记录</el-button>
            <el-button :loading="busy" @click="refresh">刷新</el-button>
            <el-button @click="openIm">打开消息页</el-button>
            <el-tag size="small" :type="online ? 'success' : 'info'">
              CDP {{ online ? '在线' : '离线' }}
            </el-tag>
            <el-tag v-if="polling" size="small" type="warning">轮询中</el-tag>
            <el-tag v-if="!enabled" size="small" type="info">未启用</el-tag>
            <span class="cs-actions__meta">角标 {{ lastBadge ?? '—' }} · 间隔 {{ intervalSec }}s</span>
          </div>

          <div class="cs-console">
            <div class="cs-console__head">
              <span class="cs-console__dot" :class="{ on: polling || online }" />
              <span class="cs-console__title">AI 客服实时执行控制台 · CDP 浏览器实例</span>
              <span class="cs-console__pill">Profile: {{ csProfileLabel }}</span>
              <span class="cs-console__meta">
                角标 {{ lastBadge ?? '—' }} · {{ online ? 'CDP Online' : 'Offline' }}
                {{ polling ? ' · Polling' : '' }}
              </span>
            </div>
            <div class="cs-console__body">
              <aside class="cs-console__side">
                <div class="acq-side-stack">
                  <div class="acq-cloud" :class="`tone-${csCloudCard.tone}`">
                    <div class="acq-cloud__top">
                      <span class="acq-cloud__brand">小红书客服</span>
                      <span class="acq-cloud__batt" :class="{ on: csCloudCard.battOn }">
                        <i class="acq-cloud__batt-ico" aria-hidden="true" />
                        {{ csCloudCard.batt }}
                      </span>
                    </div>
                    <div class="acq-cloud__device" aria-hidden="true">
                      <div class="acq-cloud__frame">
                        <div class="acq-cloud__phone">
                          <span class="acq-cloud__notch" />
                          <div class="acq-cloud__grid">
                            <i /><i /><i /><i /><i /><i /><i /><i /><i />
                          </div>
                        </div>
                      </div>
                    </div>
                    <div class="acq-cloud__title">{{ csCloudCard.title }}</div>
                    <div class="acq-cloud__sub">{{ csCloudCard.sub }}</div>
                    <div class="acq-cloud__pill">{{ csCloudCard.pill }}</div>
                  </div>

                  <div class="acq-result" :class="`tone-${csResultBadge.tone}`">
                    <div class="acq-result__head">
                      <span class="acq-result__label">本次结果</span>
                      <span class="acq-result__badge">{{ csResultBadge.text }}</span>
                    </div>
                    <div class="acq-result__stats">
                      <div class="acq-result__stat">
                        <strong>{{ lastBadge ?? 0 }}</strong>
                        <span>未读角标</span>
                      </div>
                      <div class="acq-result__stat">
                        <strong>{{ csRunStats.repliesSent }}</strong>
                        <span>本轮回复</span>
                      </div>
                      <div class="acq-result__stat">
                        <strong>{{ csRunStats.sessionsTouched }}</strong>
                        <span>会话触及</span>
                      </div>
                    </div>
                    <p class="acq-result__msg">{{ csResultSummary }}</p>
                    <div class="acq-result__foot">{{ csResultFoot }}</div>
                  </div>
                </div>
              </aside>
              <div ref="csLogEl" class="cs-console__logs">
                <div
                  v-for="entry in logs"
                  :key="entry.id"
                  class="cs-line"
                  :class="[`tone-${entry.tone}`, { 'is-done': entry.tone === 'ok' }]"
                >
                  <span class="cs-line__time">[{{ entry.time }}]</span>
                  <span class="cs-line__tag">[{{ entry.tag }}]</span>
                  <span class="cs-line__msg">{{ entry.message }}</span>
                </div>
                <div v-if="!logs.length" class="cs-line tone-neutral cs-line--empty">
                  <span class="cs-line__time">[--:--:--]</span>
                  <span class="cs-line__tag">[待命]</span>
                  <span class="cs-line__msg">控制台空闲 · 点击「开始自动回复」后实时输出执行链路</span>
                </div>
              </div>
            </div>
          </div>
        </div>

        <div v-show="activeTab === 'acq'" class="ops-pane" role="tabpanel">
          <div class="cs-actions">
            <el-button type="primary" :disabled="!online || acqRunning" @click="startAcq">
              {{ acqRunning ? '获客执行中…' : '开始获客' }}
            </el-button>
            <el-button v-if="!acqRunning" @click="stopAcqHint">停止说明</el-button>
            <el-button v-else type="warning" plain @click="resetAcqStuck">解除卡住</el-button>
            <el-button @click="acqConfigOpen = true">配置</el-button>
            <el-button @click="openAcqHistory">历史</el-button>
            <el-button :loading="busy" @click="refresh">刷新</el-button>
            <el-tag size="small" :type="online ? 'success' : 'info'">
              CDP {{ online ? '在线' : '离线' }}
            </el-tag>
            <el-tag v-if="acqRunning" size="small" type="warning">执行中</el-tag>
            <span class="cs-actions__meta">
              搜索「{{ acqSearchKeyword || '—' }}」· 评论「{{ acqCommentKeyword || '—' }}」· {{ acqSort }}
            </span>
          </div>
          <div class="cs-console">
            <div class="cs-console__head">
              <span class="cs-console__dot" :class="{ on: acqRunning || online }" />
              <span class="cs-console__title">AI 获客实时执行控制台 · CDP 浏览器实例</span>
              <span class="cs-console__pill">Profile: {{ csProfileLabel }}</span>
              <span class="cs-console__meta">
                {{ online ? 'CDP Online' : 'Offline' }}
                {{ acqRunning ? ' · Running' : '' }}
              </span>
            </div>
            <div class="cs-console__body">
              <aside class="cs-console__side">
                <div class="acq-side-stack">
                  <!-- 云端风控机位卡 -->
                  <div class="acq-cloud" :class="`tone-${acqCloudCard.tone}`">
                    <div class="acq-cloud__top">
                      <span class="acq-cloud__brand">小红书获客</span>
                      <span class="acq-cloud__batt" :class="{ on: acqCloudCard.battOn }">
                        <i class="acq-cloud__batt-ico" aria-hidden="true" />
                        {{ acqCloudCard.batt }}
                      </span>
                    </div>
                    <div class="acq-cloud__device" aria-hidden="true">
                      <div class="acq-cloud__frame">
                        <div class="acq-cloud__phone">
                          <span class="acq-cloud__notch" />
                          <div class="acq-cloud__grid">
                            <i /><i /><i /><i /><i /><i /><i /><i /><i />
                          </div>
                        </div>
                      </div>
                    </div>
                    <div class="acq-cloud__title">{{ acqCloudCard.title }}</div>
                    <div class="acq-cloud__sub">{{ acqCloudCard.sub }}</div>
                    <div class="acq-cloud__pill">{{ acqCloudCard.pill }}</div>
                  </div>

                  <!-- 本次结果（延用上一版） -->
                  <div class="acq-result" :class="`tone-${acqResultBadge.tone}`">
                    <div class="acq-result__head">
                      <span class="acq-result__label">本次结果</span>
                      <span class="acq-result__badge">{{ acqResultBadge.text }}</span>
                    </div>
                    <div class="acq-result__stats">
                      <div class="acq-result__stat">
                        <strong>{{ acqLastResult?.notesOpened ?? 0 }}</strong>
                        <span>打开笔记</span>
                      </div>
                      <div class="acq-result__stat">
                        <strong>{{ acqLastResult?.commentsMatched ?? 0 }}</strong>
                        <span>匹配评论</span>
                      </div>
                      <div class="acq-result__stat">
                        <strong>{{ acqLastResult?.repliesSent ?? 0 }}</strong>
                        <span>已发送</span>
                      </div>
                    </div>
                    <div class="acq-result__kw" v-if="acqLastResult">
                      <div>搜：{{ acqLastResult.keyword || '—' }}</div>
                      <div>评：{{ acqLastResult.commentKeyword || '—' }}</div>
                    </div>
                    <p class="acq-result__msg">{{ acqResultSummary }}</p>
                    <div class="acq-result__foot">{{ acqResultFoot }}</div>
                  </div>
                </div>
              </aside>
              <div ref="acqLogEl" class="cs-console__logs">
                <div
                  v-for="entry in acqLogs"
                  :key="entry.id"
                  class="cs-line"
                  :class="[`tone-${entry.tone}`, { 'is-done': entry.tone === 'ok' }]"
                >
                  <span class="cs-line__time">[{{ entry.time }}]</span>
                  <span class="cs-line__tag">[{{ entry.tag }}]</span>
                  <span class="cs-line__msg">{{ entry.message }}</span>
                </div>
                <div v-if="!(acqLogs?.length)" class="cs-line tone-neutral cs-line--empty">
                  <span class="cs-line__time">[--:--:--]</span>
                  <span class="cs-line__tag">[待命]</span>
                  <span class="cs-line__msg">控制台空闲 · 点击「开始获客」后实时输出执行链路</span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>

    <el-dialog
      v-model="acqHistoryOpen"
      title="获客历史评论记录"
      width="920px"
      destroy-on-close
      append-to-body
      class="acq-history-dialog"
    >
      <div class="acq-history__toolbar">
        <span class="acq-history__hint">
          记录本 Profile 下获客已匹配并尝试回复的评论（含发送成败）
        </span>
        <div class="acq-history__actions">
          <el-button size="small" :loading="acqHistoryLoading" @click="loadAcqHistory">
            刷新
          </el-button>
          <el-button
            size="small"
            type="danger"
            plain
            :disabled="!acqHistoryRows.length"
            @click="clearAcqHistory"
          >
            清空
          </el-button>
        </div>
      </div>
      <el-table
        v-loading="acqHistoryLoading"
        :data="acqHistoryRows"
        size="small"
        stripe
        max-height="480"
        empty-text="暂无历史记录，跑一轮获客并成功匹配评论后会出现在这里"
      >
        <el-table-column label="时间" width="158">
          <template #default="{ row }">
            {{ formatAcqHistoryTime(row.createdAt) }}
          </template>
        </el-table-column>
        <el-table-column prop="authorName" label="评论用户" width="110" show-overflow-tooltip>
          <template #default="{ row }">
            {{ row.authorName || '—' }}
          </template>
        </el-table-column>
        <el-table-column prop="commentText" label="原评论" min-width="180" show-overflow-tooltip />
        <el-table-column prop="replyText" label="回复内容" min-width="140" show-overflow-tooltip />
        <el-table-column prop="commentKeyword" label="命中词" width="88" show-overflow-tooltip />
        <el-table-column prop="searchKeyword" label="搜索词" width="88" show-overflow-tooltip />
        <el-table-column prop="noteTitle" label="笔记" min-width="120" show-overflow-tooltip>
          <template #default="{ row }">
            {{ row.noteTitle || `第 ${Number(row.noteIndex) + 1} 篇` }}
          </template>
        </el-table-column>
        <el-table-column label="状态" width="72" align="center">
          <template #default="{ row }">
            <el-tag size="small" :type="row.ok ? 'success' : 'danger'" effect="plain">
              {{ row.ok ? '已发' : '失败' }}
            </el-tag>
          </template>
        </el-table-column>
      </el-table>
      <template #footer>
        <el-button @click="acqHistoryOpen = false">关闭</el-button>
      </template>
    </el-dialog>

    <el-dialog
      v-model="acqConfigOpen"
      title="AI 获客配置"
      width="780px"
      destroy-on-close
      append-to-body
      class="cs-config-dialog acq-config-dialog"
    >
      <p class="lede cs-config-dialog__lede">
        用「搜索关键词」找笔记 → 排序 → 打开笔记 → 用「评论关键词」匹配评论 → 发送自定义回复。
      </p>
      <el-form label-width="120px" size="small" :disabled="acqRunning">
        <el-form-item label="CDP Profile">
          <el-select v-model="profileId" style="width: 100%">
            <el-option
              v-for="p in profiles"
              :key="p.id"
              :label="`${p.name} · ${statuses.find((s) => s.profileId === p.id)?.cdpOnline ? '在线' : '离线'}`"
              :value="p.id"
            />
          </el-select>
        </el-form-item>
        <el-form-item label="搜索关键词" required>
          <el-input
            v-model="acqSearchKeyword"
            placeholder="用于小红书搜索框，找相关笔记"
          />
        </el-form-item>
        <el-form-item label="评论关键词" required>
          <el-input
            v-model="acqCommentKeyword"
            placeholder="如：求带@怎么学@求链接"
            type="textarea"
            :rows="2"
          />
          <p class="form-tip">多个关键词请用 <strong>@</strong> 分隔，评论含任一关键词即回复</p>
          <template v-if="showAcqProbes">
            <el-button
              style="margin-top: 8px"
              :disabled="!online || acqRunning"
              @click="probeAcqComments"
            >
              探测评论匹配
            </el-button>
            <span class="form-hint">需先在浏览器打开一篇笔记；鼠标定位评论区并下滑匹配</span>
          </template>
        </el-form-item>
        <el-form-item label="排序">
          <el-select v-model="acqSort" style="width: 200px">
            <el-option v-for="s in acqSortOptions" :key="s" :label="s" :value="s" />
          </el-select>
          <el-button
            v-if="showAcqProbes"
            style="margin-left: 8px"
            @click="probeAcqFilters"
          >
            探测筛选项
          </el-button>
        </el-form-item>
        <el-form-item label="自定义回复" required>
          <el-input
            v-model="acqReply"
            type="textarea"
            :rows="3"
            placeholder="命中评论关键词后发送的固定文案"
          />
          <template v-if="showAcqProbes">
            <el-button
              style="margin-top: 8px"
              type="warning"
              plain
              :disabled="!online || acqRunning"
              @click="probeAcqCustomReply"
            >
              探测自定义回复
            </el-button>
            <span class="form-hint">需先打开笔记；会真实点回复并发送 1 条，用于验证链路</span>
          </template>
        </el-form-item>
        <div class="acq-limits">
          <el-form-item label="处理笔记数" label-width="88px" class="acq-limits__item">
            <el-input-number v-model="acqMaxNotes" :min="1" :max="30" controls-position="right" />
          </el-form-item>
          <el-form-item label="每帖回复数" label-width="88px" class="acq-limits__item">
            <el-input-number v-model="acqMaxReplies" :min="1" :max="10" controls-position="right" />
          </el-form-item>
          <el-form-item label="评论下滑次数" label-width="100px" class="acq-limits__item">
            <el-input-number v-model="acqMaxScrolls" :min="0" :max="10" controls-position="right" />
          </el-form-item>
        </div>
      </el-form>
      <template #footer>
        <el-button @click="acqConfigOpen = false">关闭</el-button>
        <el-button type="primary" @click="saveAcqConfig">保存</el-button>
      </template>
    </el-dialog>

    <el-dialog
      v-model="csConfigOpen"
      title="AI 客服配置"
      width="560px"
      destroy-on-close
      append-to-body
      class="cs-config-dialog"
    >
      <p class="lede cs-config-dialog__lede">
        绑定已登录的 CDP Profile 与知识库，轮询未读私信：向量检索 → LLM 组织回复 → CDP 发送。
        开始后切换页面不会中断，仅点「停止」或关闭应用后才会结束。连续多条会攒批后由模型决定汇总或逐条回复。
      </p>
      <el-form label-width="108px" size="small">
        <el-form-item label="CDP Profile">
          <el-select
            v-model="profileId"
            style="width: 100%"
            :disabled="polling"
            @change="loadBinding"
          >
            <el-option
              v-for="p in profiles"
              :key="p.id"
              :label="`${p.name} · ${statuses.find((s) => s.profileId === p.id)?.cdpOnline ? '在线' : '离线'}`"
              :value="p.id"
            />
          </el-select>
        </el-form-item>
        <el-form-item label="知识库">
          <el-select v-model="selectedKbIds" multiple collapse-tags style="width: 100%">
            <el-option v-for="b in bases" :key="b.id" :label="b.name" :value="b.id" />
          </el-select>
        </el-form-item>
        <el-form-item label="系统提示词">
          <el-input v-model="systemPrompt" type="textarea" :rows="4" />
        </el-form-item>
        <el-form-item label="轮询间隔">
          <el-input-number
            :model-value="intervalSec"
            :min="3"
            :max="60"
            @update:model-value="onIntervalChange"
          />
          <span style="margin-left: 8px">秒 · 角标 {{ lastBadge ?? '—' }}</span>
        </el-form-item>
        <el-form-item label="启用">
          <el-switch v-model="enabled" :disabled="polling" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="csConfigOpen = false">关闭</el-button>
        <el-button type="primary" :loading="busy" @click="saveBinding">保存绑定</el-button>
      </template>
    </el-dialog>

    <AutoOpsChatPanel v-model="chatPanelOpen" :profile-id="profileId" />
  </div>
</template>

<style scoped>
/* 整页钉在 main 内，控制台底部贴齐窗口；日志只在内部滚 */
.auto-ops {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
  overflow: hidden;
  box-sizing: border-box;
  padding: 16px 20px 14px;
}
.auto-ops > .mf-ops__panel {
  flex: 1 1 0%;
  min-height: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
.auto-ops :deep(.mf-ops__panel-body) {
  flex: 1 1 0%;
  min-height: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  /* 底边不留白：控制台贴齐面板，圆角由面板 overflow 统一裁切 */
  padding: 12px 0 0;
  gap: 0;
}
.auto-ops__tabbar {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: 0 0 auto;
  margin: 0 14px 12px;
  border-bottom: 2px solid var(--el-border-color-light, #e4e7ed);
}
.auto-ops__tab {
  appearance: none;
  border: 0;
  background: transparent;
  padding: 0 16px;
  height: 40px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font: inherit;
  font-size: 14px;
  font-weight: 600;
  color: var(--el-text-color-regular, #606266);
  cursor: pointer;
  position: relative;
  margin-bottom: -2px;
  border-bottom: 2px solid transparent;
}
.auto-ops__tab:hover {
  color: var(--el-color-primary, #409eff);
}
.auto-ops__tab.is-active {
  color: var(--el-color-primary, #409eff);
  border-bottom-color: var(--el-color-primary, #409eff);
}
.auto-ops__tab-dot {
  margin-left: 2px;
  vertical-align: middle;
}
.ops-pane {
  display: flex;
  flex-direction: column;
  flex: 1 1 0%;
  min-height: 0;
  overflow: hidden;
}
.lede {
  margin: 0 0 14px;
  color: var(--muted);
  font-size: 13px;
}
.cs-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin: 0 14px 10px;
  flex: 0 0 auto;
  max-height: 72px;
  overflow: hidden;
}
.cs-actions__meta {
  margin-left: 4px;
  color: var(--muted);
  font-size: 12px;
  min-width: 0;
  flex: 1 1 160px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.cs-config-dialog__lede {
  margin-bottom: 16px;
}
.form-hint {
  margin-left: 10px;
  color: var(--muted);
  font-size: 12px;
}
.form-tip {
  margin: 6px 0 0;
  color: var(--muted);
  font-size: 12px;
  line-height: 1.5;
}
.form-tip strong {
  color: var(--el-color-primary);
  font-weight: 600;
}
.acq-limits {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 20px;
  align-items: center;
  width: 100%;
  margin-bottom: 10px;
}
.acq-limits__item {
  margin-bottom: 0 !important;
  margin-right: 0 !important;
  flex: 0 0 auto;
  white-space: nowrap;
  :deep(.el-form-item__label) {
    padding-right: 8px;
  }
  :deep(.el-form-item__content) {
    flex-wrap: nowrap;
    margin-left: 0 !important;
  }
  :deep(.el-input-number) {
    width: 110px;
  }
}
.acq-history__toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
}
.acq-history__hint {
  font-size: 12px;
  color: var(--muted, #6b7280);
  line-height: 1.4;
}
.acq-history__actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}
.acq-config-dialog {
  :deep(.el-dialog__body) {
    padding-right: 28px;
  }
}
/* 控制台贴齐面板底边与左右；底角跟面板 14px 圆角一致，不再叠两层圆角露白 */
.cs-console {
  flex: 1 1 0%;
  min-height: 0;
  max-height: 100%;
  display: flex;
  flex-direction: column;
  margin: 0;
  border-radius: 0;
  overflow: hidden;
  border: none;
  border-top: 1px solid #2a2f3a;
  background: #0e1117;
  box-shadow: none;
  color: #d7dde8;
}
.cs-console__head {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  padding: 10px 14px;
  background: linear-gradient(180deg, #161b24 0%, #12161e 100%);
  border-bottom: 1px solid #252b36;
  font-size: 12px;
  flex-shrink: 0;
  border-radius: 0;
}
.cs-console__dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #4b5563;
  flex-shrink: 0;
}
.cs-console__dot.on {
  background: #22c55e;
  box-shadow: 0 0 8px rgba(34, 197, 94, 0.7);
}
.cs-console__title {
  font-weight: 600;
  letter-spacing: 0.02em;
  color: #e8edf5;
}
.cs-console__pill {
  padding: 2px 10px;
  border-radius: 999px;
  background: #1c2230;
  border: 1px solid #2f3748;
  color: #9aa3b5;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
}
.cs-console__meta {
  margin-left: auto;
  color: #6b7385;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
}
.cs-console__body {
  display: flex;
  flex: 1 1 0%;
  min-height: 0;
  overflow: hidden;
  border-radius: 0;
}
.cs-console__side {
  flex: 0 0 148px;
  width: 148px;
  padding: 12px 10px;
  border-right: 1px solid #1f2530;
  background: #0c0f14;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  border-radius: 0;
}
.cs-phone {
  width: 112px;
  border-radius: 16px;
  border: 1px solid #2a3140;
  background: #141922;
  padding: 8px 6px 10px;
  text-align: center;
}
.cs-phone__notch {
  width: 36px;
  height: 4px;
  border-radius: 4px;
  background: #2c3342;
  margin: 0 auto 8px;
}
.cs-phone__screen {
  border-radius: 10px;
  background: radial-gradient(120% 80% at 50% 0%, #1a2233 0%, #0d1118 70%);
  padding: 12px 8px 16px;
  min-height: 120px;
}
.cs-phone__battery {
  font-size: 10px;
  color: #7d8799;
  margin-bottom: 18px;
}
.cs-phone__status {
  font-size: 13px;
  font-weight: 700;
  color: #f3f6fb;
  margin-bottom: 6px;
}
.cs-phone__sub {
  font-size: 10px;
  color: #8b93a6;
  line-height: 1.4;
}
.cs-phone__btn {
  margin-top: 10px;
  font-size: 10px;
  color: #9aa3b5;
  border: 1px dashed #3a4254;
  border-radius: 6px;
  padding: 4px 0;
}
.acq-side-stack {
  width: 100%;
  max-width: 128px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.acq-cloud {
  border-radius: 16px;
  border: 1px solid #2a3140;
  background: linear-gradient(165deg, #151a24 0%, #0b0e14 72%);
  padding: 10px 8px 10px;
  text-align: center;
  box-sizing: border-box;
}
.acq-cloud__top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 4px;
  margin-bottom: 12px;
  padding: 0 2px;
}
.acq-cloud__brand {
  font-size: 10px;
  color: #8b93a6;
  letter-spacing: 0.02em;
}
.acq-cloud__batt {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  font-size: 10px;
  color: #6b7385;
  font-variant-numeric: tabular-nums;
}
.acq-cloud__batt.on {
  color: #4ade80;
}
.acq-cloud__batt-ico {
  width: 14px;
  height: 8px;
  border: 1px solid currentColor;
  border-radius: 2px;
  position: relative;
  box-sizing: border-box;
  opacity: 0.9;
}
.acq-cloud__batt-ico::before {
  content: '';
  position: absolute;
  inset: 1px;
  background: currentColor;
  border-radius: 1px;
  opacity: 0.35;
}
.acq-cloud__batt.on .acq-cloud__batt-ico::before {
  opacity: 0.85;
}
.acq-cloud__batt-ico::after {
  content: '';
  position: absolute;
  right: -3px;
  top: 1.5px;
  width: 1.5px;
  height: 4px;
  background: currentColor;
  border-radius: 0 1px 1px 0;
}
.acq-cloud__device {
  display: flex;
  justify-content: center;
  margin-bottom: 10px;
}
.acq-cloud__frame {
  width: 52px;
  height: 52px;
  border-radius: 12px;
  border: 2px solid #3b82f6;
  background: rgba(59, 130, 246, 0.08);
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 0 0 1px rgba(59, 130, 246, 0.15);
}
.acq-cloud.tone-run .acq-cloud__frame {
  animation: acq-cloud-pulse 1.6s ease-in-out infinite;
}
.acq-cloud.tone-off .acq-cloud__frame {
  border-color: #3a4254;
  background: rgba(255, 255, 255, 0.03);
  box-shadow: none;
}
.acq-cloud__phone {
  width: 28px;
  height: 40px;
  border-radius: 5px;
  border: 1.5px solid #5b6578;
  background: #0e121a;
  padding: 4px 3px 3px;
  box-sizing: border-box;
}
.acq-cloud__notch {
  display: block;
  width: 10px;
  height: 2px;
  border-radius: 2px;
  background: #3a4254;
  margin: 0 auto 3px;
}
.acq-cloud__grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 2px;
}
.acq-cloud__grid i {
  display: block;
  aspect-ratio: 1;
  border-radius: 2px;
  background: #334155;
}
.acq-cloud__grid i:nth-child(1) {
  background: #60a5fa;
}
.acq-cloud__grid i:nth-child(2) {
  background: #34d399;
}
.acq-cloud__grid i:nth-child(3) {
  background: #fbbf24;
}
.acq-cloud__grid i:nth-child(4) {
  background: #f472b6;
}
.acq-cloud__grid i:nth-child(5) {
  background: #a78bfa;
}
.acq-cloud__grid i:nth-child(6) {
  background: #38bdf8;
}
.acq-cloud__title {
  font-size: 14px;
  font-weight: 700;
  color: #f3f6fb;
  line-height: 1.25;
  margin-bottom: 4px;
}
.acq-cloud__sub {
  font-size: 10px;
  color: #8b93a6;
  line-height: 1.35;
  margin-bottom: 10px;
}
.acq-cloud__pill {
  display: inline-block;
  max-width: 100%;
  font-size: 10px;
  color: #9aa3b5;
  background: #1a2030;
  border: 1px solid #2a3140;
  border-radius: 999px;
  padding: 4px 8px;
  line-height: 1.2;
}
.acq-cloud.tone-run .acq-cloud__pill {
  color: #fbbf24;
  border-color: rgba(251, 191, 36, 0.35);
  background: rgba(251, 191, 36, 0.08);
}
.acq-cloud.tone-ready .acq-cloud__pill {
  color: #9aa3b5;
}
.acq-cloud.tone-off .acq-cloud__title {
  color: #c9d1df;
}
@keyframes acq-cloud-pulse {
  0%,
  100% {
    box-shadow: 0 0 0 1px rgba(59, 130, 246, 0.2);
  }
  50% {
    box-shadow: 0 0 0 3px rgba(59, 130, 246, 0.28);
  }
}
.acq-result {
  width: 100%;
  border-radius: 12px;
  border: 1px solid #2a3140;
  background: linear-gradient(180deg, #151b26 0%, #0d1118 100%);
  padding: 10px 8px 8px;
  box-sizing: border-box;
}
.acq-result__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 4px;
  margin-bottom: 10px;
}
.acq-result__label {
  font-size: 11px;
  font-weight: 700;
  color: #e8edf7;
}
.acq-result__badge {
  font-size: 10px;
  line-height: 1;
  padding: 3px 5px;
  border-radius: 999px;
  border: 1px solid #3a4254;
  color: #9aa3b5;
  flex-shrink: 0;
}
.acq-result.tone-run .acq-result__badge {
  color: #fbbf24;
  border-color: rgba(251, 191, 36, 0.45);
  background: rgba(251, 191, 36, 0.08);
}
.acq-result.tone-ok .acq-result__badge {
  color: #4ade80;
  border-color: rgba(74, 222, 128, 0.45);
  background: rgba(74, 222, 128, 0.08);
}
.acq-result.tone-warn .acq-result__badge {
  color: #f59e0b;
  border-color: rgba(245, 158, 11, 0.45);
  background: rgba(245, 158, 11, 0.08);
}
.acq-result__stats {
  display: grid;
  gap: 6px;
  margin-bottom: 8px;
}
.acq-result__stat {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 6px 4px;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.03);
  border: 1px solid #1f2530;
}
.acq-result__stat strong {
  font-size: 18px;
  line-height: 1.1;
  color: #f3f6fb;
  font-variant-numeric: tabular-nums;
}
.acq-result__stat span {
  font-size: 10px;
  color: #8b93a6;
}
.acq-result__kw {
  font-size: 10px;
  color: #8b93a6;
  line-height: 1.45;
  margin-bottom: 6px;
  word-break: break-all;
}
.acq-result__msg {
  margin: 0;
  font-size: 10px;
  color: #aeb6c7;
  line-height: 1.4;
  min-height: 2.8em;
  word-break: break-word;
}
.acq-result__foot {
  margin-top: 8px;
  font-size: 10px;
  color: #7d8799;
  text-align: center;
  border-top: 1px dashed #2a3140;
  padding-top: 6px;
}
.cs-console__logs {
  flex: 1 1 0%;
  min-width: 0;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 16px 18px 20px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 12px;
  line-height: 1.65;
  box-sizing: border-box;
  border-radius: 0;
  background: #0e1117;
}
.cs-console__logs::-webkit-scrollbar {
  width: 8px;
}
.cs-console__logs::-webkit-scrollbar-track {
  background: transparent;
}
.cs-console__logs::-webkit-scrollbar-thumb {
  background: #3a4254;
  border-radius: 999px;
  border: 2px solid transparent;
  background-clip: padding-box;
}
.cs-console__logs::-webkit-scrollbar-thumb:hover {
  background: #525c72;
  background-clip: padding-box;
}
.cs-console__logs::-webkit-scrollbar-corner {
  background: transparent;
}
.cs-line {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  padding: 1px 0;
  color: #c9d1df;
}
.cs-line--empty {
  opacity: 0.72;
}
.cs-line__time {
  color: #6b7385;
  flex-shrink: 0;
}
.cs-line__tag {
  font-weight: 600;
  flex-shrink: 0;
}
.cs-line__msg {
  color: #d7dde8;
  word-break: break-word;
}
.cs-line.tone-neutral .cs-line__tag {
  color: #e5e7eb;
}
.cs-line.tone-step .cs-line__tag {
  color: #60a5fa;
}
.cs-line.tone-cyan .cs-line__tag {
  color: #22d3ee;
}
.cs-line.tone-gold .cs-line__tag {
  color: #fbbf24;
}
.cs-line.tone-purple .cs-line__tag {
  color: #c084fc;
}
.cs-line.tone-ok .cs-line__tag,
.cs-line.tone-ok .cs-line__msg,
.cs-line.is-done {
  color: #4ade80;
}
.cs-line.tone-ok .cs-line__time {
  color: #4ade80;
}
.cs-line.tone-warn .cs-line__tag {
  color: #f59e0b;
}
.cs-line.tone-err .cs-line__tag,
.cs-line.tone-err .cs-line__msg {
  color: #f87171;
}
/* 窄窗 / 矮窗：藏左侧手机示意，把空间留给可滚动日志 */
@media (max-width: 900px), (max-height: 700px) {
  .cs-console__side {
    display: none;
  }
  .cs-actions__meta {
    display: none;
  }
}
.auto-ops__placeholder {
  padding: 48px 16px;
  text-align: center;
  color: var(--muted);
  strong {
    display: block;
    margin-bottom: 8px;
    font-size: 16px;
    color: var(--el-text-color-primary, #303133);
  }
  p {
    margin: 0;
    font-size: 13px;
  }
}
</style>
