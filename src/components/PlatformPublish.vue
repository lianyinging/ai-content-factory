<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { Platform } from '@/types/item'
import {
  draftList,
  draftUpsert,
  parseImages,
  parseLogSteps,
  parseTags,
  publishLogAdd,
  publishLogClear,
  publishLogList,
  type DraftRow,
  type PublishLogRow,
} from '@/api/drafts'
import { mediaDisplaySrc } from '@/api/assets'
import {
  browserCdpStatusAll,
  launchBrowserCdp,
  listBrowserProfiles,
  openBrowserUrl,
  parseProfileSession,
  type BrowserProfile,
  type BrowserStatus,
} from '@/api/cdp'
import {
  formatPublishStepLabel,
  getPublishJob,
  isPublishJobRunning,
  markPublishJobSeen,
  startPublishCdpJob,
  type PublishCdpAction,
} from '@/state/publishCdp'

const props = defineProps<{ platform: Platform }>()

const route = useRoute()
const router = useRouter()
const loading = ref(false)

/** 仅「复制打开」等短操作；CDP 填表/发布走后台任务，不锁整页 */
const localBusy = ref(false)

type ListTab = 'pending' | 'platform_draft' | 'published' | 'logs'
const listTab = ref<ListTab>('pending')

const drafts = ref<DraftRow[]>([])
const logs = ref<PublishLogRow[]>([])
const profiles = ref<BrowserProfile[]>([])
const statuses = ref<BrowserStatus[]>([])
const draftId = ref('')
const profileId = ref('')
const expandedLogId = ref('')

const lastSteps = ref<string[]>([])
const lastMessage = ref('')
const lastPublished = ref(false)
const lastSavedDraft = ref(false)

type StepView = { key: string; label: string; ok: boolean }

function formatPublishStep(raw: string): StepView {
  const { label, ok } = formatPublishStepLabel(raw)
  return { key: raw, label, ok }
}

const selectedJob = computed(() =>
  draftId.value ? getPublishJob(props.platform, draftId.value) : null,
)
const detailRunning = computed(() => selectedJob.value?.status === 'running')
const liveLogEl = ref<HTMLElement | null>(null)
const liveLines = computed(() => selectedJob.value?.liveLines || [])
const showLiveLog = computed(() => liveLines.value.length > 0)
const liveLogActionLabel = computed(() => {
  const a = selectedJob.value?.action
  if (a === 'publish') return '填表并发布'
  if (a === 'saveDraft') return '保存到草稿'
  if (a === 'fill') return '仅填表'
  return 'CDP'
})

function scrollLiveLogToBottom() {
  void nextTick(() => {
    const el = liveLogEl.value
    if (el) el.scrollTop = el.scrollHeight
  })
}

const resultSteps = computed(() => lastSteps.value.map(formatPublishStep))
const resultTone = computed(() => {
  if (selectedJob.value?.status === 'error') return 'error'
  if (!lastMessage.value) return 'idle'
  if (lastPublished.value) return 'success'
  if (lastSavedDraft.value) return 'draft'
  return 'pending'
})
const resultBadge = computed(() => {
  if (selectedJob.value?.status === 'error') return '操作失败'
  if (lastPublished.value) return '已提交发布'
  if (lastSavedDraft.value) return '已暂存草稿'
  return '填表完成'
})

function draftJobRunning(id: string) {
  return isPublishJobRunning(props.platform, id)
}

function applyJobToUi() {
  const job = selectedJob.value
  if (!job) return
  if (job.status === 'running') {
    lastMessage.value = ''
    lastSteps.value = []
    lastPublished.value = false
    lastSavedDraft.value = false
    return
  }
  lastSteps.value = job.steps || []
  lastMessage.value = job.message || ''
  lastPublished.value = Boolean(job.published)
  lastSavedDraft.value = Boolean(job.savedDraft)
}

function statusLabel(status?: string) {
  if (status === 'ready') return '待操作'
  if (status === 'published') return '已发布'
  if (status === 'platform_draft') return '已存草稿'
  if (status === 'filled') return '已填表'
  return '编辑中'
}

function statusType(status?: string): 'success' | 'warning' | 'info' {
  if (status === 'published') return 'success'
  if (status === 'ready' || status === 'filled' || status === 'platform_draft') return 'warning'
  return 'info'
}

function tabOfStatus(status: string): Exclude<ListTab, 'logs'> | null {
  if (status === 'published') return 'published'
  if (status === 'platform_draft') return 'platform_draft'
  // 仅待发布/已填表出现在发布中心；editing 等留在草稿工坊
  if (status === 'ready' || status === 'filled') return 'pending'
  return null
}

function actionLabel(action: string) {
  if (action === 'publish') return '填表并发布'
  if (action === 'saveDraft') return '保存到草稿'
  if (action === 'fill') return '仅填表'
  if (action === 'copy') return '复制打开'
  return action || '操作'
}

const pendingDrafts = computed(() =>
  drafts.value.filter((d) => tabOfStatus(d.status) === 'pending'),
)
const platformDrafts = computed(() =>
  drafts.value.filter((d) => tabOfStatus(d.status) === 'platform_draft'),
)
const publishedDrafts = computed(() =>
  drafts.value.filter((d) => tabOfStatus(d.status) === 'published'),
)

function publishTabOf(status: string): Exclude<ListTab, 'logs'> {
  return tabOfStatus(status) || 'pending'
}

const tabRows = computed(() => {
  if (listTab.value === 'pending') return pendingDrafts.value
  if (listTab.value === 'platform_draft') return platformDrafts.value
  if (listTab.value === 'published') return publishedDrafts.value
  return []
})

const tabCounts = computed(() => ({
  pending: pendingDrafts.value.length,
  platform_draft: platformDrafts.value.length,
  published: publishedDrafts.value.length,
  logs: logs.value.length,
}))

const selected = computed(() => drafts.value.find((d) => d.id === draftId.value) || null)
const tags = computed(() => (selected.value ? parseTags(selected.value.tagsJson) : []))
const images = computed(() => (selected.value ? parseImages(selected.value.imageUrlsJson) : []))

const statusOf = computed(() => statuses.value.find((s) => s.profileId === profileId.value))
const cdpOnline = computed(() => Boolean(statusOf.value?.cdpOnline))

const profileSession = computed(() => {
  const p = profiles.value.find((x) => x.id === profileId.value)
  const s = parseProfileSession(p?.sessionJson)
  return props.platform === 'xhs' ? s.xhs : s.douyin
})

const checks = computed(() => {
  const d = selected.value
  return [
    { key: 'title', label: '标题', ok: Boolean(d?.title?.trim()) },
    { key: 'body', label: '正文', ok: Boolean(d?.body?.trim()) },
    { key: 'images', label: '配图', ok: images.value.length > 0 },
    { key: 'cdp', label: 'CDP 在线', ok: cdpOnline.value },
  ]
})

const canFill = computed(
  () =>
    Boolean(selected.value) &&
    checks.value.every((c) => c.key === 'cdp' || c.ok) &&
    cdpOnline.value &&
    images.value.length > 0,
)

const creatorUrl = computed(() =>
  props.platform === 'xhs'
    ? 'https://creator.xiaohongshu.com/publish/publish?source=official'
    : 'https://creator.douyin.com/creator-micro/content/upload',
)

function fmtTime(raw?: string) {
  return (raw || '').replace('T', ' ').slice(0, 16) || '—'
}

function selectDraft(row: DraftRow) {
  draftId.value = row.id
  lastMessage.value = ''
  lastSteps.value = []
  lastPublished.value = false
  lastSavedDraft.value = false
  applyJobToUi()
}

async function onDeleteDraft(row: DraftRow) {
  try {
    await ElMessageBox.confirm(
      `将「${row.title || '未命名'}」移出发布中心？内容会保留在草稿工坊。`,
      '移出发布',
      { type: 'warning', confirmButtonText: '移出', cancelButtonText: '取消' },
    )
  } catch {
    return
  }
  try {
    // 与草稿工坊共用同一条 drafts；此处只改状态，不物理删除
    await draftUpsert({
      id: row.id,
      platform: props.platform,
      title: row.title,
      body: row.body,
      tagsJson: row.tagsJson,
      imageUrlsJson: row.imageUrlsJson,
      sourceItemId: row.sourceItemId || undefined,
      status: 'editing',
    })
    if (draftId.value === row.id) {
      draftId.value = ''
      lastMessage.value = ''
      lastSteps.value = []
    }
    ElMessage.success('已移出，可在草稿工坊继续编辑')
    await refresh({ silent: true })
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

function draftsForTab(tab: Exclude<ListTab, 'logs'>) {
  if (tab === 'pending') return pendingDrafts.value
  if (tab === 'platform_draft') return platformDrafts.value
  return publishedDrafts.value
}

function onTabChange(name: string | number) {
  const tab = String(name) as ListTab
  listTab.value = tab
  if (tab === 'logs') return
  const rows = draftsForTab(tab)
  if (!rows.some((d) => d.id === draftId.value)) {
    draftId.value = rows[0]?.id || ''
    lastMessage.value = ''
    lastSteps.value = []
    lastPublished.value = false
    lastSavedDraft.value = false
  }
  applyJobToUi()
}

async function refresh(opts?: { silent?: boolean }) {
  const silent = Boolean(opts?.silent)
  if (!silent) loading.value = true
  try {
    drafts.value = await draftList(props.platform)
    logs.value = await publishLogList(props.platform, 120)
    profiles.value = await listBrowserProfiles()
    const sum = await browserCdpStatusAll()
    statuses.value = sum.profiles || []

    const qid = String(route.query.draftId || '')
    if (qid && drafts.value.some((d) => d.id === qid)) {
      draftId.value = qid
      listTab.value = publishTabOf(drafts.value.find((d) => d.id === qid)!.status)
    } else if (!draftId.value || !drafts.value.some((d) => d.id === draftId.value)) {
      const prefer =
        pendingDrafts.value[0] ||
        platformDrafts.value[0] ||
        publishedDrafts.value[0] ||
        drafts.value[0]
      draftId.value = prefer?.id || ''
      if (prefer) listTab.value = publishTabOf(prefer.status)
    }

    if (!profileId.value) {
      profileId.value =
        statuses.value.find((s) => s.cdpOnline)?.profileId || profiles.value[0]?.id || ''
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    if (!silent) loading.value = false
  }
}

function exportText(d: DraftRow) {
  const tagLine = parseTags(d.tagsJson)
    .map((t) => `#${t}`)
    .join(' ')
  return `${d.title}\n\n${d.body}\n\n${tagLine}`.trim()
}

async function recordLog(input: {
  draftId?: string
  title?: string
  action: string
  ok: boolean
  message?: string
  steps?: string[]
}) {
  try {
    await publishLogAdd({
      platform: props.platform,
      draftId: input.draftId,
      title: input.title,
      action: input.action,
      ok: input.ok,
      message: input.message,
      stepsJson: JSON.stringify(input.steps || []),
      profileId: profileId.value || undefined,
    })
    logs.value = await publishLogList(props.platform, 120)
  } catch {
    /* ignore log write failure */
  }
}

async function copyAndOpen() {
  const d = selected.value
  if (!d) return
  if (localBusy.value || detailRunning.value) return
  localBusy.value = true
  try {
    try {
      await navigator.clipboard.writeText(exportText(d))
    } catch {
      /* ignore */
    }
    if (!cdpOnline.value && profileId.value) {
      await launchBrowserCdp(profileId.value)
      await refresh({ silent: true })
    }
    await openBrowserUrl(creatorUrl.value, profileId.value || undefined)
    ElMessage.success('已复制文案并打开创作者中心')
    await recordLog({
      draftId: d.id,
      title: d.title,
      action: 'copy',
      ok: true,
      message: '已复制文案并打开创作者中心',
    })
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    ElMessage.error(msg)
    await recordLog({
      draftId: d.id,
      title: d.title,
      action: 'copy',
      ok: false,
      message: msg,
    })
  } finally {
    localBusy.value = false
  }
}

function runCdp(action: PublishCdpAction) {
  const d = selected.value
  if (!d) {
    ElMessage.warning('请选择草稿')
    return
  }
  if (detailRunning.value || draftJobRunning(d.id)) {
    ElMessage.warning('该草稿正在自动化中，可离开本页稍后再回来查看')
    return
  }
  if (!d.title.trim() || !d.body.trim()) {
    ElMessage.warning('草稿缺少标题或正文，请回草稿工坊完善')
    return
  }
  if (!images.value.length) {
    ElMessage.warning('草稿需要至少一张配图')
    return
  }
  if (!profileId.value) {
    ElMessage.warning('请先选择 CDP Profile')
    return
  }

  lastMessage.value = ''
  lastSteps.value = []
  lastPublished.value = false
  lastSavedDraft.value = false

  const started = startPublishCdpJob({
    platform: props.platform,
    draftId: d.id,
    draftTitle: d.title,
    draftBody: d.body,
    tagsJson: d.tagsJson,
    imageUrlsJson: d.imageUrlsJson,
    sourceItemId: d.sourceItemId,
    images: images.value,
    profileId: profileId.value,
    action,
  })
  if (!started.ok) {
    ElMessage.warning(started.reason || '无法启动')
    return
  }
  ElMessage.info('已在后台执行，可离开发布中心浏览其它页面')
}

async function clearLogs() {
  try {
    await ElMessageBox.confirm('清空本平台全部操作记录？此操作不可恢复。', '清空记录', {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    await publishLogClear(props.platform)
    logs.value = []
    ElMessage.success('已清空操作记录')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

function openLogDraft(log: PublishLogRow) {
  if (!log.draftId) return
  const d = drafts.value.find((x) => x.id === log.draftId)
  if (!d) {
    ElMessage.warning('对应草稿已不存在')
    return
  }
  draftId.value = d.id
  listTab.value = publishTabOf(d.status)
}

function goDrafts() {
  const path = props.platform === 'xhs' ? '/xhs/drafts' : '/douyin/drafts'
  router.push({
    path,
    query: draftId.value ? { draftId: draftId.value } : undefined,
  })
}

watch(
  () => route.query.draftId,
  (id) => {
    const qid = String(id || '')
    if (qid && drafts.value.some((d) => d.id === qid)) {
      draftId.value = qid
      listTab.value = publishTabOf(drafts.value.find((d) => d.id === qid)!.status)
      applyJobToUi()
    }
  },
)

watch(
  selectedJob,
  async (job, prev) => {
    if (!job) return
    if (job.status === 'running') {
      applyJobToUi()
      scrollLiveLogToBottom()
      return
    }
    const justFinished = prev?.status === 'running'
    applyJobToUi()
    scrollLiveLogToBottom()
    if (justFinished || !job.seen) {
      markPublishJobSeen(props.platform, job.draftId)
      await refresh({ silent: true })
      if (job.status === 'done' && job.nextStatus) {
        listTab.value = publishTabOf(job.nextStatus)
      }
      if (justFinished) {
        if (job.status === 'done') ElMessage.success(job.message || '完成')
        else ElMessage.error(job.message || '失败')
      }
    }
  },
  { deep: true },
)

watch(
  () => liveLines.value.length,
  () => scrollLiveLogToBottom(),
)

watch(draftId, () => {
  applyJobToUi()
  scrollLiveLogToBottom()
})

async function hydrateJobOnEnter() {
  applyJobToUi()
  const job = selectedJob.value
  if (!job || job.status === 'running' || job.seen) return
  markPublishJobSeen(props.platform, job.draftId)
  await refresh({ silent: true })
  if (job.status === 'done' && job.nextStatus) {
    listTab.value = publishTabOf(job.nextStatus)
  }
}

onMounted(async () => {
  await refresh()
  await hydrateJobOnEnter()
})
</script>

<template>
  <div class="publish">
    <section class="mf-ops__panel publish__panel">
      <div class="mf-ops__panel-head publish__panel-head">
        <div class="publish__head-actions">
          <el-button size="small" :loading="loading" @click="refresh()">刷新</el-button>
          <el-button size="small" @click="goDrafts">回草稿工坊</el-button>
        </div>
      </div>

      <div class="mf-ops__panel-body publish__body">
        <el-tabs v-model="listTab" class="publish__tabs" @tab-change="onTabChange">
          <el-tab-pane name="pending">
            <template #label>待操作 <em>{{ tabCounts.pending }}</em></template>
          </el-tab-pane>
          <el-tab-pane name="platform_draft">
            <template #label>已存草稿 <em>{{ tabCounts.platform_draft }}</em></template>
          </el-tab-pane>
          <el-tab-pane name="published">
            <template #label>已发布 <em>{{ tabCounts.published }}</em></template>
          </el-tab-pane>
          <el-tab-pane name="logs">
            <template #label>操作记录 <em>{{ tabCounts.logs }}</em></template>
          </el-tab-pane>
        </el-tabs>

        <!-- 内容列表 + 详情 -->
        <div v-if="listTab !== 'logs'" class="publish__split">
          <div class="publish__list" v-loading="loading">
            <div
              v-for="d in tabRows"
              :key="d.id"
              class="publish__row"
              :class="{ 'is-active': d.id === draftId, 'is-running': draftJobRunning(d.id) }"
              @click="selectDraft(d)"
            >
              <div class="publish__row-main">
                <strong>{{ d.title || '未命名' }}</strong>
                <em>{{ fmtTime(d.updatedAt) }}</em>
              </div>
              <div class="publish__row-ops" @click.stop>
                <el-tag v-if="draftJobRunning(d.id)" size="small" type="primary">进行中</el-tag>
                <el-tag v-else size="small" :type="statusType(d.status)">
                  {{ statusLabel(d.status) }}
                </el-tag>
                <el-button
                  size="small"
                  text
                  type="danger"
                  :disabled="draftJobRunning(d.id)"
                  @click="onDeleteDraft(d)"
                >
                  移出
                </el-button>
              </div>
            </div>
            <div v-if="!tabRows.length" class="mf-ops__empty mf-ops__empty--compact">
              <strong>当前分类暂无内容</strong>
              <p>可从草稿工坊完善后进入发布中心，或切换其它 Tab。</p>
              <el-button type="primary" size="small" @click="goDrafts">去草稿工坊</el-button>
            </div>
          </div>

          <div class="publish__detail" v-if="selected">
            <div class="publish__checks">
              <span
                v-for="c in checks"
                :key="c.key"
                class="publish__check"
                :class="{ 'is-ok': c.ok }"
              >
                {{ c.ok ? '✓' : '○' }} {{ c.label }}
              </span>
            </div>

            <el-form label-width="100px" size="small" class="publish__form">
              <el-form-item label="CDP Profile">
                <el-select v-model="profileId" style="width: 100%">
                  <el-option
                    v-for="p in profiles"
                    :key="p.id"
                    :label="`${p.name} · ${statuses.find((s) => s.profileId === p.id)?.cdpOnline ? '在线' : '离线'} · :${p.debugPort}`"
                    :value="p.id"
                  />
                </el-select>
                <p class="publish__hint">
                  <template v-if="profileSession?.nickname">
                    会话：{{ profileSession.nickname }}
                    <template v-if="profileSession.redId"> · {{ profileSession.redId }}</template>
                  </template>
                  <template v-else>未探测会话时可先到账号矩阵登录并「刷新会话」</template>
                </p>
              </el-form-item>
            </el-form>

            <article class="publish__preview">
              <header>
                <strong>{{ selected.title || '未命名' }}</strong>
                <el-tag size="small" :type="statusType(selected.status)">
                  {{ statusLabel(selected.status) }}
                </el-tag>
              </header>
              <p class="publish__body-text">{{ selected.body || '（无正文）' }}</p>
              <div v-if="tags.length" class="publish__tags">
                <el-tag v-for="t in tags" :key="t" size="small">#{{ t }}</el-tag>
              </div>
              <div v-if="images.length" class="publish__imgs">
              <img
                v-for="(url, i) in images"
                :key="`${url}-${i}`"
                :src="mediaDisplaySrc(url)"
                alt=""
                referrerpolicy="no-referrer"
              />
              </div>
              <p v-else class="publish__hint">暂无配图，请回草稿工坊补充后再发布。</p>
            </article>

            <div class="publish__ops-panel">
              <div class="publish__actions">
                <el-button size="small" :disabled="localBusy || detailRunning" @click="copyAndOpen">
                  {{ localBusy ? '处理中…' : '复制并打开创作者中心' }}
                </el-button>
                <el-button
                  size="small"
                  type="primary"
                  plain
                  :disabled="!canFill || detailRunning"
                  @click="runCdp('fill')"
                >
                  CDP 填表
                </el-button>
                <el-button
                  v-if="platform === 'xhs'"
                  size="small"
                  :disabled="!canFill || detailRunning"
                  @click="runCdp('saveDraft')"
                >
                  CDP 保存到草稿
                </el-button>
                <el-button
                  size="small"
                  type="primary"
                  :disabled="!canFill || detailRunning"
                  @click="runCdp('publish')"
                >
                  CDP 填表并发布
                </el-button>
                <span v-if="detailRunning" class="publish__busy-hint">
                  可离开本页浏览其它功能，回来后继续显示进度或结果
                </span>
              </div>

              <div v-if="showLiveLog" class="publish__live-log">
                <div class="publish__live-log__head">
                  <span
                    class="publish__live-log__dot"
                    :class="{ 'is-running': detailRunning }"
                  />
                  <span class="publish__live-log__title">
                    CDP 执行日志 · {{ liveLogActionLabel }}
                  </span>
                  <em v-if="detailRunning">进行中</em>
                  <em v-else>已结束</em>
                </div>
                <div ref="liveLogEl" class="publish__live-log__body">
                  <div
                    v-for="entry in liveLines"
                    :key="entry.id"
                    class="publish__live-line"
                    :class="`tone-${entry.tone}`"
                  >
                    <span class="publish__live-line__time">[{{ entry.time }}]</span>
                    <span class="publish__live-line__tag">[{{ entry.tag }}]</span>
                    <span class="publish__live-line__msg">{{ entry.message }}</span>
                  </div>
                </div>
              </div>

              <div v-if="lastMessage" class="publish__result" :class="`is-${resultTone}`">
                <div class="publish__result-head">
                  <span class="publish__result-badge">{{ resultBadge }}</span>
                  <p>{{ lastMessage }}</p>
                </div>
                <ol v-if="resultSteps.length" class="publish__result-steps">
                  <li
                    v-for="(st, i) in resultSteps"
                    :key="`${st.key}-${i}`"
                    :class="{ 'is-fail': !st.ok }"
                  >
                    <i>{{ st.ok ? '✓' : '!' }}</i>
                    <span>{{ st.label }}</span>
                  </li>
                </ol>
              </div>
            </div>
          </div>

          <div v-else class="publish__detail publish__detail--empty">
            <div class="mf-ops__empty">
              <strong>选择一条内容</strong>
              <p>在左侧列表点选后，可预览并执行 CDP 操作。</p>
            </div>
          </div>
        </div>

        <!-- 操作记录 -->
        <div v-else class="publish__logs">
          <div class="publish__logs-toolbar">
            <p>保留本机 CDP / 打开创作者中心等操作历史，便于回溯。</p>
            <el-button size="small" :disabled="!logs.length" @click="clearLogs">清空记录</el-button>
          </div>
          <div v-if="!logs.length" class="mf-ops__empty mf-ops__empty--compact">
            <strong>暂无操作记录</strong>
            <p>执行填表、暂存或发布后会自动写入这里。</p>
          </div>
          <ul v-else class="publish__log-list">
            <li v-for="log in logs" :key="log.id" class="publish__log">
              <button type="button" class="publish__log-head" @click="expandedLogId = expandedLogId === log.id ? '' : log.id">
                <div class="publish__log-main">
                  <strong>{{ log.title || '未命名' }}</strong>
                  <em>{{ fmtTime(log.createdAt) }} · {{ actionLabel(log.action) }}</em>
                </div>
                <el-tag size="small" :type="log.ok ? 'success' : 'danger'">
                  {{ log.ok ? '成功' : '失败' }}
                </el-tag>
              </button>
              <div v-if="expandedLogId === log.id" class="publish__log-body">
                <p>{{ log.message || '—' }}</p>
                <ol v-if="parseLogSteps(log.stepsJson).length" class="publish__result-steps">
                  <li
                    v-for="(st, i) in parseLogSteps(log.stepsJson).map(formatPublishStep)"
                    :key="`${log.id}-${i}`"
                    :class="{ 'is-fail': !st.ok }"
                  >
                    <i>{{ st.ok ? '✓' : '!' }}</i>
                    <span>{{ st.label }}</span>
                  </li>
                </ol>
                <el-button
                  v-if="log.draftId"
                  link
                  type="primary"
                  size="small"
                  @click="openLogDraft(log)"
                >
                  查看对应内容
                </el-button>
              </div>
            </li>
          </ul>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped lang="scss">
.publish {
  height: 100%;
  min-height: 0;

  :deep(.el-loading-mask) {
    cursor: default;
  }
}
.publish__ops-panel {
  position: relative;
  min-height: 72px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.publish__live-log {
  display: flex;
  flex-direction: column;
  height: 180px;
  border-radius: 8px;
  overflow: hidden;
  border: 1px solid #e4e7ed;
  background: linear-gradient(180deg, #1c2230 0%, #141821 100%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
}
.publish__live-log__head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  background: rgba(255, 255, 255, 0.03);
  flex-shrink: 0;

  em {
    margin-left: auto;
    font-style: normal;
    font-size: 11px;
    color: #8b93a7;
  }
}
.publish__live-log__dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #6b7280;
  flex-shrink: 0;

  &.is-running {
    background: #409eff;
    box-shadow: 0 0 0 0 rgba(64, 158, 255, 0.55);
    animation: publish-live-pulse 1.8s ease-out infinite;
  }
}
@keyframes publish-live-pulse {
  0% {
    box-shadow: 0 0 0 0 rgba(64, 158, 255, 0.45);
  }
  70% {
    box-shadow: 0 0 0 8px rgba(64, 158, 255, 0);
  }
  100% {
    box-shadow: 0 0 0 0 rgba(64, 158, 255, 0);
  }
}
.publish__live-log__title {
  font-size: 11px;
  color: #c9d1df;
  font-weight: 600;
  letter-spacing: 0.02em;
}
.publish__live-log__body {
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
.publish__live-log__body::-webkit-scrollbar {
  width: 6px;
}
.publish__live-log__body::-webkit-scrollbar-thumb {
  background: #3a4254;
  border-radius: 999px;
}
.publish__live-line {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  color: #a8b0c0;

  &.tone-ok {
    color: #6ee7a8;
  }
  &.tone-warn {
    color: #fbbf24;
  }
  &.tone-err {
    color: #f87171;
  }
  &.tone-info {
    color: #93c5fd;
  }
}
.publish__live-line__time {
  color: #6b7280;
  flex-shrink: 0;
}
.publish__live-line__tag {
  color: #9ca3af;
  flex-shrink: 0;
}
.publish__live-line__msg {
  flex: 1;
  min-width: 0;
  word-break: break-word;
}
.publish__panel {
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.publish__panel-head {
  justify-content: flex-end;
}
.publish__head-actions {
  display: flex;
  gap: 8px;
}
.publish__body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.publish__tabs {
  flex-shrink: 0;
  :deep(.el-tabs__header) {
    margin-bottom: 12px;
  }
  :deep(.el-tabs__item) {
    em {
      font-style: normal;
      margin-left: 4px;
      font-size: 12px;
      color: var(--muted, #888);
    }
    &.is-active em {
      color: var(--el-color-primary);
    }
  }
}
.publish__split {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(240px, 300px) 1fr;
  gap: 12px;
  overflow: hidden;
}
.publish__list,
.publish__detail,
.publish__logs {
  min-height: 0;
  overflow: auto;
}
.publish__list {
  border: 1px solid var(--border, #e5e7eb);
  border-radius: 10px;
  background: #fff;
}
.publish__row {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 12px 12px;
  border: 0;
  border-bottom: 1px solid var(--border, #eee);
  background: transparent;
  text-align: left;
  cursor: pointer;
  &.is-active {
    background: color-mix(in srgb, var(--el-color-primary) 8%, transparent);
  }
  &.is-running {
    background: color-mix(in srgb, var(--el-color-primary) 6%, transparent);
  }
}
.publish__row-main {
  min-width: 0;
  flex: 1;
  strong {
    display: block;
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  em {
    font-style: normal;
    font-size: 11px;
    color: var(--muted, #888);
  }
}
.publish__row-ops {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}
.publish__detail {
  padding: 4px 4px 12px;
  &--empty {
    display: grid;
    place-items: center;
  }
}
.publish__checks {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 14px;
  margin-bottom: 12px;
}
.publish__check {
  font-size: 12px;
  color: var(--muted, #888);
  &.is-ok {
    color: var(--el-color-success);
  }
}
.publish__form {
  max-width: 560px;
}
.publish__hint {
  margin: 6px 0 0;
  font-size: 12px;
  color: var(--muted, #888);
}
.publish__preview {
  margin: 8px 0 16px;
  padding: 14px 16px;
  border: 1px solid var(--border, #e5e7eb);
  border-radius: 10px;
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 8px;
    strong {
      font-size: 16px;
    }
  }
}
.publish__body-text {
  margin: 0 0 10px;
  white-space: pre-wrap;
  line-height: 1.6;
  font-size: 13px;
  color: var(--text, #222);
}
.publish__tags {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 10px;
}
.publish__imgs {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(96px, 1fr));
  gap: 8px;
  img {
    width: 100%;
    aspect-ratio: 1;
    object-fit: cover;
    border-radius: 8px;
    background: #f3f4f6;
  }
}
.publish__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}
.publish__busy-hint {
  font-size: 12px;
  color: var(--muted, #888);
}
.publish__result {
  margin-top: 16px;
  padding: 14px 16px;
  border-radius: 10px;
  border: 1px solid var(--border, #e5e7eb);
  background: #f8fafc;

  &.is-success {
    border-color: color-mix(in srgb, var(--el-color-success) 35%, transparent);
    background: color-mix(in srgb, var(--el-color-success) 8%, #fff);
  }
  &.is-draft {
    border-color: color-mix(in srgb, var(--el-color-primary) 35%, transparent);
    background: color-mix(in srgb, var(--el-color-primary) 8%, #fff);
  }
  &.is-pending {
    border-color: color-mix(in srgb, var(--el-color-warning) 35%, transparent);
    background: color-mix(in srgb, var(--el-color-warning) 8%, #fff);
  }
  &.is-error {
    border-color: color-mix(in srgb, var(--el-color-danger) 35%, transparent);
    background: color-mix(in srgb, var(--el-color-danger) 8%, #fff);
  }
}
.publish__result-head {
  display: flex;
  flex-direction: column;
  gap: 6px;
  p {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
  }
}
.publish__result-badge {
  align-self: flex-start;
  font-size: 11px;
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 999px;
  color: var(--el-color-warning);
  background: color-mix(in srgb, var(--el-color-warning) 14%, transparent);

  .is-success & {
    color: var(--el-color-success);
    background: color-mix(in srgb, var(--el-color-success) 14%, transparent);
  }
  .is-draft & {
    color: var(--el-color-primary);
    background: color-mix(in srgb, var(--el-color-primary) 14%, transparent);
  }
  .is-error & {
    color: var(--el-color-danger);
    background: color-mix(in srgb, var(--el-color-danger) 14%, transparent);
  }
}
.publish__result-steps {
  list-style: none;
  margin: 12px 0 0;
  padding: 0;
  display: grid;
  gap: 6px;

  li {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    i {
      flex: 0 0 18px;
      width: 18px;
      height: 18px;
      border-radius: 50%;
      display: grid;
      place-items: center;
      font-style: normal;
      font-size: 11px;
      font-weight: 700;
      color: #fff;
      background: var(--el-color-success);
    }
    &.is-fail {
      color: var(--el-color-danger);
      i {
        background: var(--el-color-danger);
      }
    }
  }
}
.publish__logs-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
  p {
    margin: 0;
    font-size: 12px;
    color: var(--muted, #888);
  }
}
.publish__log-list {
  list-style: none;
  margin: 0;
  padding: 0;
  border: 1px solid var(--border, #e5e7eb);
  border-radius: 10px;
  overflow: hidden;
}
.publish__log {
  border-bottom: 1px solid var(--border, #eee);
  &:last-child {
    border-bottom: 0;
  }
}
.publish__log-head {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 12px 14px;
  border: 0;
  background: transparent;
  text-align: left;
  cursor: pointer;
  &:hover {
    background: #fafafa;
  }
}
.publish__log-main {
  min-width: 0;
  strong {
    display: block;
    font-size: 13px;
  }
  em {
    font-style: normal;
    font-size: 11px;
    color: var(--muted, #888);
  }
}
.publish__log-body {
  padding: 0 14px 12px;
  p {
    margin: 0 0 8px;
    font-size: 13px;
    line-height: 1.5;
  }
}

@media (max-width: 900px) {
  .publish__split {
    grid-template-columns: 1fr;
  }
}
</style>
