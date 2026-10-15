<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Grid, List, Plus } from '@element-plus/icons-vue'
import { openUrl } from '@tauri-apps/plugin-opener'
import type { LibraryGroup, LibraryRow, Platform, ViralAnalysis } from '@/types/item'
import { mediaDisplaySrc } from '@/api/assets'
import {
  libraryDelete,
  libraryGroupCreate,
  libraryGroupDelete,
  libraryGroupList,
  libraryGroupRename,
  libraryList,
  libraryMoveToGroup,
} from '@/api/library'
import { libraryToDrafts } from '@/api/drafts'
import { competitorAdd } from '@/api/competitors'
import { useLibraryAnalyzeStore } from '@/state/libraryAnalyze'

function coverSrc(row: LibraryRow): string {
  const local = String(row.coverLocalPath || '').trim()
  const remote = String(row.coverUrl || '').trim()
  return mediaDisplaySrc(local || remote)
}

function parseAnalysis(raw?: string | null): ViralAnalysis | null {
  const s = String(raw || '').trim()
  if (!s) return null
  try {
    const o = JSON.parse(s) as Partial<ViralAnalysis>
    return {
      summary: String(o.summary || ''),
      hooks: Array.isArray(o.hooks) ? o.hooks.map(String) : [],
      structure: String(o.structure || ''),
      suggestions: Array.isArray(o.suggestions) ? o.suggestions.map(String) : [],
      score: Math.max(0, Math.min(100, Number(o.score) || 0)),
    }
  } catch {
    return null
  }
}

const props = defineProps<{ platform: Platform; label: string }>()
const router = useRouter()
const analyzeStore = useLibraryAnalyzeStore()

type ViewMode = 'card' | 'table'
/** null=全部；''=未分组；string=组 id */
type GroupFilter = string | null

function viewKey() {
  return `ai-factory.libraryView.${props.platform}`
}

const loading = ref(false)
const allRows = ref<LibraryRow[]>([])
const noteQuery = ref('')
const appliedQuery = ref('')
const multiSelect = ref(false)
const selected = ref<string[]>([])
const viewMode = ref<ViewMode>('card')
const coverBroken = ref<Record<string, boolean>>({})

const groups = ref<LibraryGroup[]>([])
const totalCount = ref(0)
const ungroupedCount = ref(0)
const ungroupedAnalysisJson = ref('')
const ungroupedAnalyzedAt = ref('')
const activeGroup = ref<GroupFilter>(null)

const analyzeOpen = ref(false)
const analyzeTitle = ref('')
const analyzeScope = ref<'item' | 'group'>('item')
const analyzeTargetId = ref('')
const analyzeResult = ref<ViralAnalysis | null>(null)
const analyzeCachedAt = ref('')

const analyzeLoading = computed(() =>
  analyzeStore.isBusy(props.platform, analyzeScope.value, analyzeTargetId.value),
)

const analyzeBtnLabel = computed(() => {
  if (analyzeLoading.value) return '分析中…'
  return analyzeResult.value ? '重新分析' : '开始分析'
})

type GroupCtxMenu =
  | { open: false }
  | {
      open: true
      x: number
      y: number
      kind: 'blank' | 'all' | 'ungrouped' | 'group'
      group?: LibraryGroup
    }

const groupMenu = ref<GroupCtxMenu>({ open: false })
/** 刚打开右键菜单后忽略一次 click，避免立刻被关掉 */
let groupMenuIgnoreClickUntil = 0

const rows = computed(() => {
  const q = appliedQuery.value.trim().toLowerCase()
  if (!q) return allRows.value
  return allRows.value.filter((r) => {
    const title = (r.title || '').toLowerCase()
    const author = (r.authorName || '').toLowerCase()
    return title.includes(q) || author.includes(q) || r.itemId.toLowerCase().includes(q)
  })
})

const activeGroupLabel = computed(() => {
  if (activeGroup.value === null) return '全部'
  if (activeGroup.value === '') return '未分组'
  return groups.value.find((g) => g.id === activeGroup.value)?.name || '分组'
})

const canAnalyzeGroup = computed(
  () => activeGroup.value !== null && rows.value.length > 0,
)

function closeGroupMenu() {
  groupMenu.value = { open: false }
}

function clampMenuPos(x: number, y: number, itemCount: number) {
  const pad = 8
  const w = 168
  const h = Math.max(40, itemCount * 36 + 12)
  const maxX = Math.max(pad, window.innerWidth - w - pad)
  const maxY = Math.max(pad, window.innerHeight - h - pad)
  return {
    x: Math.min(Math.max(pad, x), maxX),
    y: Math.min(Math.max(pad, y), maxY),
  }
}

function openGroupCtx(
  ev: MouseEvent,
  kind: 'blank' | 'all' | 'ungrouped' | 'group',
  group?: LibraryGroup,
) {
  ev.preventDefault()
  ev.stopPropagation()
  const count =
    kind === 'group' ? 4 : kind === 'ungrouped' ? 3 : kind === 'all' ? 2 : 1
  const { x, y } = clampMenuPos(ev.clientX, ev.clientY, count)
  groupMenuIgnoreClickUntil = Date.now() + 280
  groupMenu.value = { open: true, x, y, kind, group }
}

function onGroupMenuWindowPointerDown(ev: Event) {
  if (!groupMenu.value.open) return
  if (Date.now() < groupMenuIgnoreClickUntil) return
  const el = ev.target as Node | null
  const menuEl = document.querySelector('.library-ctx-menu')
  if (menuEl && el && menuEl.contains(el)) return
  closeGroupMenu()
}

function onGroupMenuKeydown(ev: KeyboardEvent) {
  if (ev.key === 'Escape') closeGroupMenu()
}

async function loadGroups() {
  const r = await libraryGroupList(props.platform)
  groups.value = r.groups
  totalCount.value = r.totalCount
  ungroupedCount.value = r.ungroupedCount
  ungroupedAnalysisJson.value = r.ungroupedAnalysisJson || ''
  ungroupedAnalyzedAt.value = r.ungroupedAnalyzedAt || ''
}

async function loadAll() {
  loading.value = true
  try {
    await loadGroups()
    allRows.value = await libraryList(props.platform, activeGroup.value)
    appliedQuery.value = noteQuery.value
    const ids = new Set(allRows.value.map((r) => r.itemId))
    selected.value = selected.value.filter((id) => ids.has(id))
    if (
      activeGroup.value &&
      activeGroup.value !== '' &&
      !groups.value.some((g) => g.id === activeGroup.value)
    ) {
      activeGroup.value = null
      allRows.value = await libraryList(props.platform, null)
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
  }
}

function onSearch() {
  appliedQuery.value = noteQuery.value
}

async function selectGroup(g: GroupFilter) {
  if (g !== null && typeof g !== 'string') return
  if (activeGroup.value === g) return
  activeGroup.value = g
  await loadAll()
}

function toggleMultiSelect() {
  multiSelect.value = !multiSelect.value
  if (!multiSelect.value) selected.value = []
}

function isSelected(id: string) {
  return selected.value.includes(id)
}

function toggleSelect(id: string) {
  if (!multiSelect.value) return
  if (selected.value.includes(id)) {
    selected.value = selected.value.filter((x) => x !== id)
  } else {
    selected.value = [...selected.value, id]
  }
}

async function createGroup() {
  try {
    const { value } = await ElMessageBox.prompt('分组名称', '新建分组', {
      inputPattern: /\S+/,
      inputErrorMessage: '请输入名称',
      confirmButtonText: '创建',
    })
    const name = String(value || '').trim()
    if (!name) return
    await libraryGroupCreate(props.platform, name)
    ElMessage.success('已创建分组')
    await loadGroups()
  } catch (e) {
    if (e === 'cancel' || e === 'close') return
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function renameGroup(g: LibraryGroup) {
  try {
    const { value } = await ElMessageBox.prompt('新名称', '重命名分组', {
      inputValue: g.name,
      inputPattern: /\S+/,
      inputErrorMessage: '请输入名称',
    })
    const name = String(value || '').trim()
    if (!name || name === g.name) return
    await libraryGroupRename(props.platform, g.id, name)
    ElMessage.success('已重命名')
    await loadGroups()
  } catch (e) {
    if (e === 'cancel' || e === 'close') return
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function deleteGroup(g: LibraryGroup) {
  try {
    await ElMessageBox.confirm(
      `删除分组「${g.name}」？组内内容将移回未分组。`,
      '确认',
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await libraryGroupDelete(props.platform, g.id)
    ElMessage.success('已删除分组')
    if (activeGroup.value === g.id) activeGroup.value = null
    await loadAll()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function moveSelectedTo(groupId: string) {
  const ids = [...selected.value]
  if (!ids.length) return
  try {
    await libraryMoveToGroup(props.platform, ids, groupId)
    ElMessage.success(`已移动 ${ids.length} 条`)
    selected.value = []
    multiSelect.value = false
    await loadAll()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function moveOneTo(row: LibraryRow, groupId: string) {
  try {
    await libraryMoveToGroup(props.platform, [row.itemId], groupId)
    ElMessage.success('已移动')
    await loadAll()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function removeOne(row: LibraryRow) {
  try {
    await ElMessageBox.confirm(`删除「${row.title || row.itemId}」？`, '确认', {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    await libraryDelete(props.platform, row.itemId)
    ElMessage.success('已删除')
    selected.value = selected.value.filter((id) => id !== row.itemId)
    await loadAll()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function toDrafts(ids: string[]) {
  if (!ids.length) return
  loading.value = true
  try {
    const r = await libraryToDrafts(props.platform, ids)
    ElMessage.success(`已转入草稿 ${r.created} 条`)
    selected.value = []
    multiSelect.value = false
    router.push(props.platform === 'xhs' ? '/xhs/drafts' : '/douyin/drafts')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
  }
}

async function toDraftOne(row: LibraryRow) {
  await toDrafts([row.itemId])
}

async function monitorOne(row: LibraryRow) {
  if (props.platform !== 'xhs') {
    ElMessage.info('抖音竞品监控后续接入')
    return
  }
  if (!row.url) {
    ElMessage.warning('缺少笔记链接')
    return
  }
  try {
    await competitorAdd({
      noteUrl: row.url,
      noteId: row.itemId,
      title: row.title,
      authorName: row.authorName,
      coverUrl: row.coverUrl,
      likes: row.likes,
      collects: row.collects,
      comments: row.comments,
      shares: row.shares,
    })
    ElMessage.success('已加入竞品监控')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function removeSelected() {
  const ids = [...selected.value]
  if (!ids.length) return
  try {
    await ElMessageBox.confirm(`删除选中的 ${ids.length} 条内容？`, '确认', {
      type: 'warning',
    })
  } catch {
    return
  }
  loading.value = true
  try {
    for (const id of ids) {
      await libraryDelete(props.platform, id)
    }
    ElMessage.success(`已删除 ${ids.length} 条`)
    selected.value = []
    multiSelect.value = false
    await loadAll()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
  }
}

async function openLink(url: string) {
  if (!url) return
  try {
    await openUrl(url)
  } catch {
    window.open(url, '_blank')
  }
}

function fmt(n: number) {
  if (n >= 10000) return `${(n / 10000).toFixed(1)}万`
  return String(n ?? 0)
}

function onCoverError(id: string) {
  coverBroken.value = { ...coverBroken.value, [id]: true }
}

function goDiscovery() {
  router.push(props.platform === 'xhs' ? '/xhs/search' : '/douyin/search')
}

function goSettings() {
  router.push('/settings')
}

function rowToAnalyzeItem(row: LibraryRow) {
  return {
    title: row.title,
    authorName: row.authorName,
    likes: row.likes,
    collects: row.collects,
    comments: row.comments,
    shares: row.shares,
    desc: row.desc,
  }
}

function collectAnalyzeItems(scope: 'item' | 'group', targetId: string) {
  if (scope === 'item') {
    return allRows.value
      .filter((r) => r.itemId === targetId)
      .map(rowToAnalyzeItem)
  }
  return [...rows.value]
    .sort((a, b) => b.likes + b.collects - (a.likes + a.collects))
    .slice(0, 12)
    .map(rowToAnalyzeItem)
}

function openAnalyzeDrawer(opts: {
  scope: 'item' | 'group'
  title: string
  targetId: string
  cached?: ViralAnalysis | null
  cachedAt?: string
  /** 无缓存时是否立刻入队后台分析 */
  autoStart?: boolean
}) {
  analyzeScope.value = opts.scope
  analyzeTitle.value = opts.title
  analyzeTargetId.value = opts.targetId
  analyzeResult.value = opts.cached || null
  analyzeCachedAt.value = opts.cachedAt || ''
  analyzeOpen.value = true
  if (!opts.cached && opts.autoStart !== false) void enqueueAnalyze()
}

function openItemAnalyze(row: LibraryRow) {
  const cached = parseAnalysis(row.analysisJson)
  openAnalyzeDrawer({
    scope: 'item',
    title: row.title || row.itemId,
    targetId: row.itemId,
    cached,
    cachedAt: row.analyzedAt || '',
    autoStart: !cached,
  })
}

function openGroupAnalyze(target?: GroupFilter) {
  closeGroupMenu()
  // 未显式传入时用当前组；忽略误传入的 MouseEvent 等非字符串值
  const gFilter: GroupFilter =
    target === undefined || (typeof target !== 'string' && target !== null)
      ? activeGroup.value
      : target
  if (gFilter === null) return

  let cached: ViralAnalysis | null = null
  let cachedAt = ''
  let title = '本组'
  if (gFilter === '') {
    cached = parseAnalysis(ungroupedAnalysisJson.value)
    cachedAt = ungroupedAnalyzedAt.value
    title = '未分组'
  } else {
    const g = groups.value.find((x) => x.id === gFilter)
    cached = parseAnalysis(g?.analysisJson)
    cachedAt = g?.analyzedAt || ''
    title = g?.name || '本组'
  }

  const open = () => {
    openAnalyzeDrawer({
      scope: 'group',
      title: `分组「${title}」`,
      targetId: gFilter,
      cached,
      cachedAt,
      autoStart: !cached,
    })
  }

  // 切到该组再入队，保证 items 快照正确
  if (activeGroup.value !== gFilter) {
    void selectGroup(gFilter).then(open)
    return
  }
  open()
}

async function ctxSelectAndClose(g: GroupFilter) {
  closeGroupMenu()
  await selectGroup(g)
}

async function ctxRenameGroup(g: LibraryGroup) {
  closeGroupMenu()
  await renameGroup(g)
}

async function ctxDeleteGroup(g: LibraryGroup) {
  closeGroupMenu()
  await deleteGroup(g)
}

async function ctxCreateGroup() {
  closeGroupMenu()
  await createGroup()
}

function enqueueAnalyze() {
  if (analyzeLoading.value) {
    ElMessage.info('该目标正在分析中')
    return
  }
  const items = collectAnalyzeItems(analyzeScope.value, analyzeTargetId.value)
  const r = analyzeStore.enqueue({
    platform: props.platform,
    scope: analyzeScope.value,
    targetId: analyzeTargetId.value,
    title: analyzeTitle.value,
    items,
  })
  if (!r.ok) {
    ElMessage.warning(r.reason || '无法开始分析')
    if (r.reason?.includes('没有可分析')) {
      /* empty */
    } else if (/未配置|LLM|api.?key/i.test(r.reason || '')) {
      ElMessage.info('请先在设置中配置文本模型（如 DeepSeek）')
    }
  }
}

function applyPendingOpen() {
  const pending = analyzeStore.takePendingOpen(props.platform)
  if (!pending) return

  if (pending.scope === 'group' && pending.targetId !== activeGroup.value) {
    void selectGroup(pending.targetId).then(() => {
      openAnalyzeDrawer({
        scope: pending.scope,
        title: pending.title,
        targetId: pending.targetId,
        cached: pending.result,
        cachedAt: pending.analyzedAt,
        autoStart: false,
      })
      if (pending.error) ElMessage.error(pending.error)
    })
    return
  }

  openAnalyzeDrawer({
    scope: pending.scope,
    title: pending.title,
    targetId: pending.targetId,
    cached: pending.result,
    cachedAt: pending.analyzedAt,
    autoStart: false,
  })
  if (pending.error) ElMessage.error(pending.error)
}

function syncJobToDrawer() {
  if (!analyzeOpen.value) return
  const job = analyzeStore.findJob(
    props.platform,
    analyzeScope.value,
    analyzeTargetId.value,
  )
  if (!job) return
  if (job.status === 'done' && job.result) {
    analyzeResult.value = job.result
    analyzeCachedAt.value =
      job.finishedAt != null
        ? new Date(job.finishedAt).toISOString().slice(0, 19).replace('T', ' ')
        : analyzeCachedAt.value
    if (job.scope === 'item') {
      const row = allRows.value.find((r) => r.itemId === job.targetId)
      if (row) {
        row.analysisJson = JSON.stringify(job.result)
        row.analyzedAt = analyzeCachedAt.value
      }
    } else {
      void loadGroups()
    }
  }
}

watch(viewMode, (mode) => {
  localStorage.setItem(viewKey(), mode)
})

watch(
  () => analyzeStore.jobs.value,
  () => syncJobToDrawer(),
  { deep: true },
)

watch(
  () => analyzeStore.pendingOpen.value,
  (p) => {
    if (p && p.platform === props.platform) applyPendingOpen()
  },
)

onMounted(() => {
  const saved = localStorage.getItem(viewKey())
  if (saved === 'card' || saved === 'table') viewMode.value = saved
  else viewMode.value = 'card'
  void loadAll().then(() => applyPendingOpen())
  window.addEventListener('pointerdown', onGroupMenuWindowPointerDown, true)
  window.addEventListener('keydown', onGroupMenuKeydown)
  window.addEventListener('resize', closeGroupMenu)
})

onUnmounted(() => {
  window.removeEventListener('pointerdown', onGroupMenuWindowPointerDown, true)
  window.removeEventListener('keydown', onGroupMenuKeydown)
  window.removeEventListener('resize', closeGroupMenu)
})
</script>

<template>
  <div class="library">
    <section class="mf-ops__panel mf-ops__discover">
      <div class="mf-ops__discover-filters">
        <div class="mf-ops__filter-row">
          <div class="mf-ops__filter-item">
            <label>关键词</label>
            <el-input
              v-model="noteQuery"
              size="small"
              clearable
              placeholder="搜索标题、作者…"
              style="width: 220px"
              @keydown.enter.prevent="onSearch"
            />
          </div>
          <div class="mf-ops__filter-item mf-ops__filter-item--action">
            <el-button type="primary" size="small" @click="onSearch">搜索</el-button>
            <el-button size="small" :loading="loading" @click="loadAll">刷新</el-button>
            <el-button
              size="small"
              :type="multiSelect ? 'primary' : 'default'"
              plain
              @click="toggleMultiSelect"
            >
              {{
                multiSelect
                  ? `取消多选${selected.length ? `（${selected.length}）` : ''}`
                  : '多选'
              }}
            </el-button>
            <el-dropdown
              v-if="multiSelect && selected.length"
              trigger="click"
              @command="moveSelectedTo"
            >
              <el-button size="small">移入分组</el-button>
              <template #dropdown>
                <el-dropdown-menu>
                  <el-dropdown-item command="">未分组</el-dropdown-item>
                  <el-dropdown-item
                    v-for="g in groups"
                    :key="g.id"
                    :command="g.id"
                  >
                    {{ g.name }}
                  </el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>
            <el-button
              v-if="multiSelect && selected.length"
              size="small"
              type="primary"
              @click="toDrafts(selected)"
            >
              转草稿
            </el-button>
            <el-button
              v-if="multiSelect && selected.length"
              size="small"
              type="danger"
              plain
              @click="removeSelected"
            >
              删除
            </el-button>
            <div class="library-view-toggle" role="group" aria-label="视图切换">
              <button
                type="button"
                class="library-view-btn"
                :class="{ 'is-active': viewMode === 'card' }"
                title="卡片"
                @click="viewMode = 'card'"
              >
                <el-icon><Grid /></el-icon>
                <span>卡片</span>
              </button>
              <button
                type="button"
                class="library-view-btn"
                :class="{ 'is-active': viewMode === 'table' }"
                title="表格"
                @click="viewMode = 'table'"
              >
                <el-icon><List /></el-icon>
                <span>表格</span>
              </button>
            </div>
          </div>
        </div>
      </div>
    </section>

    <div class="library-body">
      <aside class="library-groups" @contextmenu="openGroupCtx($event, 'blank')">
        <div class="library-groups__head">
          <strong>分组</strong>
          <el-button link type="primary" size="small" :icon="Plus" @click="createGroup">
            新建
          </el-button>
        </div>
        <button
          type="button"
          class="library-groups__item"
          :class="{ 'is-active': activeGroup === null }"
          @click="selectGroup(null)"
          @contextmenu="openGroupCtx($event, 'all')"
        >
          <span>全部</span>
          <span class="library-groups__count">{{ totalCount }}</span>
        </button>
        <button
          type="button"
          class="library-groups__item"
          :class="{ 'is-active': activeGroup === '' }"
          @click="selectGroup('')"
          @contextmenu="openGroupCtx($event, 'ungrouped')"
        >
          <span>未分组</span>
          <span class="library-groups__count">{{ ungroupedCount }}</span>
        </button>
        <div class="library-groups__divider" />
        <div v-if="!groups.length" class="library-groups__empty">暂无自定义分组</div>
        <div
          v-for="g in groups"
          :key="g.id"
          class="library-groups__row"
          @contextmenu="openGroupCtx($event, 'group', g)"
        >
          <button
            type="button"
            class="library-groups__item"
            :class="{ 'is-active': activeGroup === g.id }"
            @click="selectGroup(g.id)"
            @contextmenu="openGroupCtx($event, 'group', g)"
          >
            <span class="library-groups__name">{{ g.name }}</span>
            <span class="library-groups__count">{{ g.count }}</span>
          </button>
          <el-dropdown
            trigger="click"
            @command="(cmd: string) => (cmd === 'rename' ? renameGroup(g) : deleteGroup(g))"
          >
            <button type="button" class="library-groups__more" @click.stop @contextmenu.prevent>
              ···
            </button>
            <template #dropdown>
              <el-dropdown-menu>
                <el-dropdown-item command="rename">重命名</el-dropdown-item>
                <el-dropdown-item command="delete" divided>删除</el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
        </div>
      </aside>

      <section class="mf-ops__panel mf-ops__discover-results library-main" v-loading="loading">
        <div class="mf-ops__panel-head">
          <div class="library-head-left">
            <strong>内容库 · {{ activeGroupLabel }}</strong>
            <span class="mf-ops__result-meta">{{ rows.length }} 条</span>
          </div>
          <el-button
            v-if="canAnalyzeGroup"
            size="small"
            type="primary"
            plain
            :loading="
              activeGroup !== null &&
              analyzeStore.isBusy(platform, 'group', activeGroup)
            "
            :disabled="
              activeGroup !== null &&
              analyzeStore.isBusy(platform, 'group', activeGroup)
            "
            @click="openGroupAnalyze()"
          >
            {{
              activeGroup !== null &&
              analyzeStore.isBusy(platform, 'group', activeGroup)
                ? '分析中…'
                : '分析本组'
            }}
          </el-button>
        </div>

        <div class="mf-ops__panel-body">
          <template v-if="rows.length">
            <div v-if="viewMode === 'card'" class="mf-ops__note-grid">
              <article
                v-for="row in rows"
                :key="row.itemId"
                class="note-card"
                :class="{
                  'is-selected': multiSelect && isSelected(row.itemId),
                  'is-selectable': multiSelect,
                }"
                @click="multiSelect ? toggleSelect(row.itemId) : undefined"
              >
                <label v-if="multiSelect" class="note-card__check" @click.stop>
                  <el-checkbox
                    :model-value="isSelected(row.itemId)"
                    @change="toggleSelect(row.itemId)"
                    @click.stop
                  />
                </label>
                <div class="note-card__cover">
                  <img
                    v-if="coverSrc(row) && !coverBroken[row.itemId]"
                    :src="coverSrc(row)"
                    :alt="row.title"
                    loading="lazy"
                    referrerpolicy="no-referrer"
                    @error="onCoverError(row.itemId)"
                  />
                  <div v-else class="note-card__cover-empty">无封面</div>
                </div>
                <div class="note-card__body">
                  <h3 class="note-card__title">{{ row.title || '未命名' }}</h3>
                  <p class="note-card__author">{{ row.authorName || '未知作者' }}</p>
                  <div class="note-card__metrics">
                    <span>赞 {{ fmt(row.likes) }}</span>
                    <span>藏 {{ fmt(row.collects) }}</span>
                    <span>评 {{ fmt(row.comments) }}</span>
                    <span>转 {{ fmt(row.shares) }}</span>
                  </div>
                  <div v-if="!multiSelect" class="note-card__el-actions" @click.stop>
                    <el-button
                      size="small"
                      type="primary"
                      plain
                      :loading="analyzeStore.isBusy(platform, 'item', row.itemId)"
                      :disabled="analyzeStore.isBusy(platform, 'item', row.itemId)"
                      @click="openItemAnalyze(row)"
                    >
                      {{
                        analyzeStore.isBusy(platform, 'item', row.itemId)
                          ? '分析中…'
                          : '分析'
                      }}
                    </el-button>
                    <el-button
                      size="small"
                      type="primary"
                      plain
                      :disabled="platform !== 'xhs'"
                      @click="monitorOne(row)"
                    >
                      监控
                    </el-button>
                    <el-button size="small" @click="toDraftOne(row)">转草稿</el-button>
                    <el-dropdown trigger="click" @command="(gid: string) => moveOneTo(row, gid)">
                      <el-button size="small">分组</el-button>
                      <template #dropdown>
                        <el-dropdown-menu>
                          <el-dropdown-item command="">未分组</el-dropdown-item>
                          <el-dropdown-item
                            v-for="g in groups"
                            :key="g.id"
                            :command="g.id"
                          >
                            {{ g.name }}
                          </el-dropdown-item>
                        </el-dropdown-menu>
                      </template>
                    </el-dropdown>
                    <el-button v-if="row.url" size="small" @click="openLink(row.url)">
                      打开
                    </el-button>
                    <el-button size="small" type="danger" plain @click="removeOne(row)">
                      删除
                    </el-button>
                  </div>
                </div>
              </article>
            </div>

            <el-table v-else :data="rows" empty-text="暂无内容">
              <el-table-column v-if="multiSelect" width="48">
                <template #default="{ row }">
                  <el-checkbox
                    :model-value="isSelected(row.itemId)"
                    @change="toggleSelect(row.itemId)"
                  />
                </template>
              </el-table-column>
              <el-table-column label="封面" width="80">
                <template #default="{ row }">
                  <img
                    v-if="coverSrc(row)"
                    class="thumb"
                    :src="coverSrc(row)"
                    alt=""
                    referrerpolicy="no-referrer"
                  />
                  <span v-else class="muted">—</span>
                </template>
              </el-table-column>
              <el-table-column prop="title" label="标题" min-width="200" show-overflow-tooltip />
              <el-table-column prop="authorName" label="作者" width="120" show-overflow-tooltip />
              <el-table-column label="互动" width="200">
                <template #default="{ row }">
                  赞{{ fmt(row.likes) }} / 藏{{ fmt(row.collects) }} / 评{{ fmt(row.comments) }}
                </template>
              </el-table-column>
              <el-table-column prop="savedAt" label="入库时间" width="170" />
              <el-table-column v-if="!multiSelect" label="操作" width="200" fixed="right">
                <template #default="{ row }">
                  <el-button
                    link
                    type="primary"
                    :disabled="analyzeStore.isBusy(platform, 'item', row.itemId)"
                    @click="openItemAnalyze(row)"
                  >
                    {{
                      analyzeStore.isBusy(platform, 'item', row.itemId)
                        ? '分析中…'
                        : '分析'
                    }}
                  </el-button>
                  <el-button link type="primary" @click="openLink(row.url)">打开</el-button>
                  <el-button link type="danger" @click="removeOne(row)">删除</el-button>
                </template>
              </el-table-column>
            </el-table>
          </template>

          <div v-else class="mf-ops__empty mf-ops__empty--compact">
            <strong>{{ activeGroup === null ? '内容库为空' : '该分组暂无内容' }}</strong>
            <p>
              {{
                appliedQuery.trim()
                  ? '没有匹配的内容，试试其他关键词。'
                  : activeGroup !== null
                    ? '可将笔记移入此分组，或切换到「全部」查看。'
                    : '在发现页点「入库」，将内容保存到内容库。'
              }}
            </p>
            <el-button
              v-if="!appliedQuery.trim() && activeGroup === null"
              size="small"
              type="primary"
              @click="goDiscovery"
            >
              {{ platform === 'xhs' ? '去笔记发现' : '去作品发现' }}
            </el-button>
          </div>
        </div>
      </section>
    </div>

    <div
      v-if="groupMenu.open"
      class="library-ctx-menu"
      :style="{ left: `${groupMenu.x}px`, top: `${groupMenu.y}px` }"
      @click.stop
      @contextmenu.prevent
    >
      <template v-if="groupMenu.kind === 'blank'">
        <button type="button" @click="ctxCreateGroup">新建分组</button>
      </template>
      <template v-else-if="groupMenu.kind === 'all'">
        <button type="button" @click="ctxSelectAndClose(null)">查看全部</button>
        <button type="button" @click="ctxCreateGroup">新建分组</button>
      </template>
      <template v-else-if="groupMenu.kind === 'ungrouped'">
        <button type="button" @click="ctxSelectAndClose('')">打开未分组</button>
        <button
          type="button"
          :disabled="ungroupedCount <= 0"
          @click="openGroupAnalyze('')"
        >
          分析本组
        </button>
        <button type="button" @click="ctxCreateGroup">新建分组</button>
      </template>
      <template v-else-if="groupMenu.kind === 'group' && groupMenu.group">
        <button type="button" @click="ctxSelectAndClose(groupMenu.group!.id)">
          打开
        </button>
        <button type="button" @click="ctxRenameGroup(groupMenu.group!)">
          重命名
        </button>
        <button
          type="button"
          :disabled="groupMenu.group.count <= 0"
          @click="openGroupAnalyze(groupMenu.group!.id)"
        >
          分析本组
        </button>
        <button
          type="button"
          class="is-danger"
          @click="ctxDeleteGroup(groupMenu.group!)"
        >
          删除
        </button>
      </template>
    </div>

    <el-drawer
      v-model="analyzeOpen"
      :title="`爆款分析 · ${analyzeTitle}`"
      size="420px"
      destroy-on-close
    >
      <div v-loading="analyzeLoading" class="analyze-drawer">
        <div v-if="analyzeResult" class="analyze-drawer__body">
          <div class="analyze-score">
            <el-tag type="success" effect="dark">参考分 {{ analyzeResult.score }}</el-tag>
            <span v-if="analyzeCachedAt" class="analyze-cached">{{ analyzeCachedAt }}</span>
          </div>
          <section v-if="analyzeResult.summary">
            <h4>为什么火</h4>
            <p>{{ analyzeResult.summary }}</p>
          </section>
          <section v-if="analyzeResult.hooks.length">
            <h4>爆点 / 钩子</h4>
            <ul>
              <li v-for="(h, i) in analyzeResult.hooks" :key="i">{{ h }}</li>
            </ul>
          </section>
          <section v-if="analyzeResult.structure">
            <h4>内容结构</h4>
            <p>{{ analyzeResult.structure }}</p>
          </section>
          <section v-if="analyzeResult.suggestions.length">
            <h4>创作建议</h4>
            <ul>
              <li v-for="(s, i) in analyzeResult.suggestions" :key="i">{{ s }}</li>
            </ul>
          </section>
        </div>
        <div v-else-if="analyzeLoading" class="analyze-drawer__empty">
          <p>后台分析进行中，可关闭本页继续浏览，完成后会消息通知。</p>
        </div>
        <div v-else class="analyze-drawer__empty">
          <p>暂无分析结果。需已配置文本模型（如 DeepSeek）。</p>
          <el-button size="small" @click="goSettings">去设置</el-button>
        </div>
        <div class="analyze-drawer__foot">
          <el-button
            type="primary"
            :loading="analyzeLoading"
            :disabled="analyzeLoading"
            @click="enqueueAnalyze()"
          >
            {{ analyzeBtnLabel }}
          </el-button>
        </div>
      </div>
    </el-drawer>
  </div>
</template>

<style scoped lang="scss">
.library {
  display: flex;
  flex-direction: column;
  gap: 12px;
  flex: 1 1 auto;
  height: 100%;
  min-height: 0;
  min-width: 0;
  width: 100%;
  max-width: 100%;
  overflow: hidden;
  box-sizing: border-box;
}

.library :deep(.mf-ops__discover) {
  flex: 0 0 auto;
  width: 100%;
  max-width: 100%;
  min-width: 0;
  box-sizing: border-box;
}

.library-body {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  gap: 12px;
  overflow: hidden;
}

.library-groups {
  flex: 0 0 180px;
  width: 180px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 10px 8px;
  border-radius: 10px;
  background: #fff;
  border: 1px solid var(--el-border-color-lighter, #ebeef5);
  overflow: auto;
}

.library-groups__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 2px 6px 8px;
  font-size: 13px;
}

.library-groups__item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  padding: 8px 10px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text, #1f2329);
  font-size: 13px;
  font-family: inherit;
  text-align: left;
  cursor: pointer;

  &:hover {
    background: var(--soft, #f2f4f7);
  }

  &.is-active {
    background: var(--el-color-primary-light-9, #ecf5ff);
    color: var(--el-color-primary);
    font-weight: 600;
  }
}

.library-groups__name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.library-groups__count {
  flex-shrink: 0;
  font-size: 12px;
  color: var(--muted, #8a919f);
  font-weight: 400;
}

.library-groups__divider {
  height: 1px;
  margin: 6px 8px;
  background: var(--el-border-color-lighter, #ebeef5);
}

.library-groups__empty {
  padding: 8px 10px;
  font-size: 12px;
  color: var(--muted, #8a919f);
}

.library-groups__row {
  display: flex;
  align-items: center;
  gap: 2px;

  .library-groups__item {
    flex: 1;
    min-width: 0;
  }
}

.library-groups__more {
  flex-shrink: 0;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--muted, #8a919f);
  cursor: pointer;
  font-size: 14px;
  line-height: 1;

  &:hover {
    background: var(--soft, #f2f4f7);
    color: var(--text, #1f2329);
  }
}

.library-main {
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.library :deep(.mf-ops__discover-results) {
  width: 100%;
  max-width: 100%;
  min-width: 0;
  box-sizing: border-box;
  flex: 1 1 auto;
  min-height: 0;
}

.library-head-left {
  display: flex;
  align-items: baseline;
  gap: 10px;
  min-width: 0;
}

.library-view-toggle {
  display: inline-grid;
  grid-template-columns: 1fr 1fr;
  gap: 2px;
  padding: 3px;
  border-radius: 8px;
  background: var(--soft, #f2f4f7);
  margin-left: 4px;
}

.library-view-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  height: 28px;
  padding: 0 10px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--muted, #8a919f);
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;

  &.is-active {
    background: #fff;
    color: var(--text, #1f2329);
    font-weight: 600;
    box-shadow: 0 1px 2px rgba(15, 23, 42, 0.08);
  }
}

.thumb {
  width: 48px;
  height: 64px;
  object-fit: cover;
  border-radius: 4px;
  background: var(--soft);
}

.muted {
  color: var(--muted);
}

.analyze-drawer {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 240px;
}

.analyze-drawer__body {
  flex: 1 1 auto;
  overflow: auto;
  padding-right: 4px;

  section {
    margin-bottom: 18px;
  }

  h4 {
    margin: 0 0 8px;
    font-size: 13px;
    color: var(--muted, #8a919f);
    font-weight: 600;
  }

  p {
    margin: 0;
    font-size: 14px;
    line-height: 1.65;
    color: var(--text, #1f2329);
  }

  ul {
    margin: 0;
    padding-left: 18px;
    font-size: 14px;
    line-height: 1.7;
  }
}

.analyze-score {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 16px;
}

.analyze-cached {
  font-size: 12px;
  color: var(--muted, #8a919f);
}

.analyze-drawer__empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 12px;
  padding-top: 24px;
  color: var(--muted, #8a919f);
  font-size: 13px;
}

.analyze-drawer__foot {
  flex: 0 0 auto;
  padding-top: 16px;
  border-top: 1px solid var(--el-border-color-lighter, #ebeef5);
}

.library-ctx-menu {
  position: fixed;
  z-index: 3000;
  min-width: 148px;
  max-width: min(220px, calc(100vw - 16px));
  padding: 6px;
  border-radius: 10px;
  border: 1px solid var(--el-border-color-lighter, #ebeef5);
  background: #fff;
  box-shadow: 0 10px 30px rgba(15, 23, 42, 0.12);
  display: flex;
  flex-direction: column;
  box-sizing: border-box;

  button {
    border: none;
    background: transparent;
    text-align: left;
    padding: 8px 10px;
    border-radius: 6px;
    font-size: 13px;
    font-family: inherit;
    white-space: nowrap;
    cursor: pointer;
    color: var(--text, #1f2329);

    &:hover:not(:disabled) {
      background: #f3f4f6;
    }

    &:disabled {
      opacity: 0.45;
      cursor: not-allowed;
    }

    &.is-danger {
      color: #ef4444;
    }
  }
}
</style>
