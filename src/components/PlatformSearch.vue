<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import ItemCard from '@/components/ItemCard.vue'
import type { ContentItem, Platform } from '@/types/item'
import { authList, itemSearch, reportCatbusError } from '@/api/catbus'
import {
  clearRecentKeywords,
  loadRecentKeywords,
  removeRecentKeyword,
  saveRecentKeyword,
} from '@/utils/searchHistory'

const props = defineProps<{
  platform: Platform
  label: string
  showLengthRange?: boolean
}>()

interface AccountOpt {
  account: string
  nickname: string
}

type RevealPhase = 'idle' | 'fetching' | 'revealing' | 'done' | 'error'

/** 每页条数；首页前 N 张逐条揭示，其余一次进内存，翻页直接展示 */
const PAGE_SIZE = 10
const STEP_REVEAL = PAGE_SIZE

const query = ref('')
const sort = ref('general')
const type = ref('all')
const time = ref('all')
const length = ref('all')
const range = ref('all')
const account = ref('')
const accounts = ref<AccountOpt[]>([])
const busy = ref(false)
const searched = ref(false)
const recentKeywords = ref<string[]>([])

const phase = ref<RevealPhase>('idle')
const allItems = ref<ContentItem[]>([])
/** 已揭示到内存中的条数（分析动画用） */
const visibleCount = ref(0)
const currentPage = ref(1)
const resultsBodyEl = ref<HTMLElement | null>(null)

/** catbus 服务端翻页游标 */
const nextCursor = ref<string | null>(null)
const hasMore = ref(false)
/** 翻页时拉下一页云端结果 */
const pageFetching = ref(false)

let revealTimer: ReturnType<typeof setTimeout> | null = null
let revealGen = 0
/** 取消进行中的翻页拉取 */
let pageFetchGen = 0
let skSubTimer: ReturnType<typeof setInterval> | null = null

const FETCH_SUBS = ['检索笔记', '拉取封面', '同步数据']
const REVEAL_SUBS = ['解析内容', '整理卡片', '匹配素材']
const MORE_SUBS = ['续翻云端', '拉取下一页', '同步结果']

/** 骨架封面辅文案轮播下标 */
const skSubIndex = ref(0)

const xhsSortOptions = [
  { value: 'general', label: '综合' },
  { value: 'latest', label: '最新' },
  { value: 'popular', label: '最多点赞' },
  { value: 'comments', label: '最多评论' },
  { value: 'collects', label: '最多收藏' },
]

const dySortOptions = [
  { value: 'general', label: '综合' },
  { value: 'popular', label: '最热' },
  { value: 'latest', label: '最新' },
]

const filteredRecent = computed(() => {
  const q = query.value.trim().toLowerCase()
  if (!q) return recentKeywords.value
  return recentKeywords.value.filter((k) => k.toLowerCase().includes(q))
})

const loadedCount = computed(() => allItems.value.length)
/** 分页器展示总量：还有更多时多预留一页，让「下一页」可点 */
const pagerTotal = computed(() => {
  const n = loadedCount.value
  if (hasMore.value) return n + PAGE_SIZE
  return n
})
const totalPages = computed(() => Math.max(1, Math.ceil(pagerTotal.value / PAGE_SIZE)))

const pageStart = computed(() => (currentPage.value - 1) * PAGE_SIZE)

/** 当前页要展示的真卡 */
const pageItems = computed(() => {
  if (phase.value === 'fetching') return []
  if (phase.value === 'revealing') {
    // 分析阶段固定在第 1 页，按揭示进度出卡
    const end = Math.min(visibleCount.value, PAGE_SIZE)
    return allItems.value.slice(0, end)
  }
  return allItems.value.slice(pageStart.value, pageStart.value + PAGE_SIZE)
})

/** 当前页剩余骨架（首搜揭示 / 翻页拉取中） */
const pendingSkeleton = computed(() => {
  if (phase.value === 'fetching') return PAGE_SIZE
  if (phase.value === 'revealing' && currentPage.value === 1) {
    return Math.max(0, PAGE_SIZE - pageItems.value.length)
  }
  if (pageFetching.value && pageItems.value.length < PAGE_SIZE) {
    return Math.max(0, PAGE_SIZE - pageItems.value.length)
  }
  return 0
})

/** 骨架封面主文案 */
const skeletonLabel = computed(() => {
  if (pageFetching.value) return '加载更多中'
  if (phase.value === 'fetching') return 'AI 云端采集中'
  if (phase.value === 'revealing') return 'AI 云端分析中'
  return '处理中'
})

const skeletonSubs = computed(() => {
  if (pageFetching.value) return MORE_SUBS
  if (phase.value === 'fetching') return FETCH_SUBS
  if (phase.value === 'revealing') return REVEAL_SUBS
  return FETCH_SUBS
})

const skeletonSub = computed(() => {
  const list = skeletonSubs.value
  if (!list.length) return ''
  return list[skSubIndex.value % list.length] ?? ''
})

const resultMeta = computed(() => {
  const loaded = loadedCount.value
  if (phase.value === 'fetching') return '云端采集中…'
  if (phase.value === 'revealing') {
    return `AI 云端分析中 · 已揭示 ${visibleCount.value}/${loaded}`
  }
  if (pageFetching.value) return `加载第 ${currentPage.value} 页… · 已加载 ${loaded} 条`
  if (phase.value === 'done' && loaded) {
    const more = hasMore.value ? ' · 还有更多' : ''
    return `分析完成 · 已加载 ${loaded} 条 · 第 ${currentPage.value}/${totalPages.value} 页${more}`
  }
  if (phase.value === 'error') return '分析失败'
  if (searched.value && !loaded) return '0 条'
  return searched.value ? `${loaded} 条` : '—'
})

const showGrid = computed(
  () =>
    phase.value === 'fetching' ||
    phase.value === 'revealing' ||
    pageFetching.value ||
    (phase.value === 'done' && allItems.value.length > 0),
)

const showPager = computed(
  () =>
    phase.value === 'done' &&
    (loadedCount.value > PAGE_SIZE || hasMore.value || pageFetching.value),
)

function applySearchPage(page: { cursor: string | null; hasMore: boolean }) {
  nextCursor.value = page.cursor
  hasMore.value = page.hasMore
}

function appendItems(incoming: ContentItem[]) {
  if (!incoming.length) return
  const seen = new Set(allItems.value.map((x) => x.id))
  const extra: ContentItem[] = []
  for (const it of incoming) {
    if (!it.id || seen.has(it.id)) continue
    seen.add(it.id)
    extra.push(it)
  }
  if (extra.length) allItems.value = allItems.value.concat(extra)
}

function searchOpts(cursor?: string | null) {
  return {
    query: query.value,
    cursor: cursor || undefined,
    sort: sort.value,
    type: type.value,
    time: time.value,
    length: props.showLengthRange ? length.value : undefined,
    range: props.showLengthRange ? range.value : undefined,
    account: account.value || undefined,
  }
}

async function scrollResultsTop() {
  await nextTick()
  const el = resultsBodyEl.value
  if (el) el.scrollTop = 0
}

/** 确保已加载足够条数覆盖目标页；必要时用 --cursor 拉下一页 */
async function ensureLoadedForPage(page: number, gen: number): Promise<boolean> {
  const need = page * PAGE_SIZE
  while (allItems.value.length < need && hasMore.value && nextCursor.value) {
    if (gen !== pageFetchGen) return false
    const res = await itemSearch(props.platform, searchOpts(nextCursor.value))
    if (gen !== pageFetchGen) return false
    if (!res.ok) {
      await reportCatbusError(res.envelope, res.error || '加载更多失败')
      applySearchPage({ cursor: null, hasMore: false })
      return false
    }
    applySearchPage(res.page)
    appendItems(res.items)
    // 本页空结果但声称还有更多：停，避免死循环
    if (!res.items.length) {
      applySearchPage({ cursor: null, hasMore: false })
      break
    }
  }
  return true
}

async function onPageChange(page: number) {
  if (phase.value !== 'done' || pageFetching.value) return

  const needFetch =
    page * PAGE_SIZE > allItems.value.length && hasMore.value && !!nextCursor.value
  if (needFetch) {
    const gen = ++pageFetchGen
    pageFetching.value = true
    currentPage.value = page
    void scrollResultsTop()
    try {
      await ensureLoadedForPage(page, gen)
      if (gen !== pageFetchGen) return
    } catch (e) {
      if (gen !== pageFetchGen) return
      await reportCatbusError(e, '加载更多失败')
    } finally {
      if (gen === pageFetchGen) pageFetching.value = false
    }
  }

  const loaded = allItems.value.length
  const maxLoadedPage = Math.max(1, Math.ceil(loaded / PAGE_SIZE) || 1)
  if (!loaded || (page - 1) * PAGE_SIZE >= loaded) {
    currentPage.value = maxLoadedPage
  } else {
    currentPage.value = page
  }
  void scrollResultsTop()
}

function clearRevealTimer() {
  if (revealTimer != null) {
    clearTimeout(revealTimer)
    revealTimer = null
  }
}

function stopReveal() {
  revealGen += 1
  clearRevealTimer()
}

function sleep(ms: number, gen: number) {
  return new Promise<void>((resolve) => {
    revealTimer = setTimeout(() => {
      revealTimer = null
      if (gen === revealGen) resolve()
    }, ms)
  })
}

function stepDelayMs() {
  return 1000 + Math.floor(Math.random() * 500)
}

async function revealItems(list: ContentItem[], gen: number) {
  const total = list.length
  if (!total) {
    if (gen !== revealGen) return
    phase.value = 'done'
    visibleCount.value = 0
    currentPage.value = 1
    busy.value = false
    return
  }

  visibleCount.value = 0
  currentPage.value = 1
  phase.value = 'revealing'

  const stepN = Math.min(STEP_REVEAL, total)
  await sleep(350, gen)
  if (gen !== revealGen) return

  for (let i = 1; i <= stepN; i++) {
    if (gen !== revealGen) return
    visibleCount.value = i
    if (i < stepN) {
      await sleep(stepDelayMs(), gen)
      if (gen !== revealGen) return
    }
  }

  // 其余一次进入内存，可翻页查看（不逐条动画）
  if (total > stepN) {
    await sleep(400, gen)
    if (gen !== revealGen) return
    visibleCount.value = total
  }

  if (gen !== revealGen) return
  phase.value = 'done'
  busy.value = false
}

async function loadAccounts() {
  try {
    const env = await authList(props.platform)
    const data = env.data
    const list: unknown[] = Array.isArray(data)
      ? data
      : data && typeof data === 'object' && Array.isArray((data as { accounts?: unknown[] }).accounts)
        ? (data as { accounts: unknown[] }).accounts
        : []
    accounts.value = list
      .map((x) => {
        if (!x || typeof x !== 'object') return null
        const r = x as Record<string, unknown>
        const user =
          r.user && typeof r.user === 'object' ? (r.user as Record<string, unknown>) : {}
        const acct = String(r.account || r.name || '')
        if (!acct) return null
        return {
          account: acct,
          nickname: String(user.nickname || user.name || r.nickname || acct),
        } satisfies AccountOpt
      })
      .filter(Boolean) as AccountOpt[]
    if (!account.value && accounts.value[0]) account.value = accounts.value[0].account
  } catch {
    /* optional */
  }
}

function reloadHistory() {
  recentKeywords.value = loadRecentKeywords(props.platform)
}

/** 仅允许通过「搜索」按钮触发，禁止回车提交 */
async function onSearch() {
  if (!query.value.trim()) {
    ElMessage.warning('请输入关键词')
    return
  }
  if (!account.value && accounts.value.length) {
    ElMessage.warning('请选择搜索账号')
    return
  }

  stopReveal()
  pageFetchGen += 1
  pageFetching.value = false
  const gen = ++revealGen

  busy.value = true
  searched.value = true
  phase.value = 'fetching'
  allItems.value = []
  visibleCount.value = 0
  currentPage.value = 1
  nextCursor.value = null
  hasMore.value = false
  void scrollResultsTop()

  try {
    // 不传 --limit：取服务端一页，后续翻页用 page.cursor
    const res = await itemSearch(props.platform, searchOpts())
    if (gen !== revealGen) return
    if (!res.ok) {
      await reportCatbusError(res.envelope, res.error || '搜索失败')
      allItems.value = []
      visibleCount.value = 0
      applySearchPage({ cursor: null, hasMore: false })
      phase.value = 'error'
      busy.value = false
      return
    }
    allItems.value = res.items
    applySearchPage(res.page)
    recentKeywords.value = saveRecentKeyword(props.platform, query.value)
    await revealItems(res.items, gen)
  } catch (e) {
    if (gen !== revealGen) return
    await reportCatbusError(e, '搜索失败')
    allItems.value = []
    visibleCount.value = 0
    applySearchPage({ cursor: null, hasMore: false })
    phase.value = 'error'
    busy.value = false
  }
}

function onKeywordKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter') {
    e.preventDefault()
    e.stopPropagation()
  }
}

function pickRecent(kw: string) {
  query.value = kw
}

function onRemoveRecent(kw: string) {
  recentKeywords.value = removeRecentKeyword(props.platform, kw)
}

function onClearRecent() {
  clearRecentKeywords(props.platform)
  recentKeywords.value = []
}

function querySearch(term: string, cb: (list: { value: string }[]) => void) {
  const q = (term || '').trim().toLowerCase()
  const list = (q
    ? recentKeywords.value.filter((k) => k.toLowerCase().includes(q))
    : recentKeywords.value
  ).map((value) => ({ value }))
  cb(list)
}

function stopSkSubRotate() {
  if (skSubTimer != null) {
    clearInterval(skSubTimer)
    skSubTimer = null
  }
}

function startSkSubRotate() {
  stopSkSubRotate()
  skSubIndex.value = 0
  skSubTimer = setInterval(() => {
    skSubIndex.value += 1
  }, 1400)
}

watch(pendingSkeleton, (n) => {
  if (n > 0) startSkSubRotate()
  else stopSkSubRotate()
})

watch(skeletonSubs, () => {
  skSubIndex.value = 0
})

onMounted(() => {
  reloadHistory()
  loadAccounts()
})

onUnmounted(() => {
  stopReveal()
  stopSkSubRotate()
  pageFetchGen += 1
})
</script>

<template>
  <div class="discover">
    <section class="mf-ops__panel mf-ops__discover">
      <div class="mf-ops__discover-filters">
        <div class="mf-ops__filter-row">
          <div class="mf-ops__filter-item">
            <label>搜索账号</label>
            <el-select v-model="account" placeholder="选择账号" size="small" style="width: 140px">
              <el-option
                v-for="a in accounts"
                :key="a.account"
                :label="a.nickname"
                :value="a.account"
              />
            </el-select>
          </div>
          <div class="mf-ops__filter-item">
            <label>关键词</label>
            <el-autocomplete
              v-model="query"
              size="small"
              placeholder="输入搜索关键词"
              clearable
              style="width: 200px"
              :fetch-suggestions="querySearch"
              :trigger-on-focus="true"
              value-key="value"
              @keydown.enter.prevent
              @keydown="onKeywordKeydown"
            />
          </div>
          <div class="mf-ops__filter-item">
            <label>排序</label>
            <el-select v-model="sort" size="small" style="width: 110px">
              <el-option
                v-for="o in platform === 'xhs' ? xhsSortOptions : dySortOptions"
                :key="o.value"
                :label="o.label"
                :value="o.value"
              />
            </el-select>
          </div>
          <div class="mf-ops__filter-item">
            <label>类型</label>
            <el-select v-model="type" size="small" style="width: 96px">
              <el-option label="全部" value="all" />
              <el-option label="视频" value="video" />
              <el-option label="图文" value="image" />
            </el-select>
          </div>
          <template v-if="showLengthRange">
            <div class="mf-ops__filter-item">
              <label>时间</label>
              <el-select v-model="time" size="small" style="width: 100px">
                <el-option label="不限" value="all" />
                <el-option label="一天" value="day" />
                <el-option label="一周" value="week" />
                <el-option label="半年" value="half_year" />
              </el-select>
            </div>
            <div class="mf-ops__filter-item">
              <label>时长</label>
              <el-select v-model="length" size="small" style="width: 90px">
                <el-option label="不限" value="all" />
                <el-option label="短" value="short" />
                <el-option label="中" value="medium" />
                <el-option label="长" value="long" />
              </el-select>
            </div>
          </template>
          <div class="mf-ops__filter-item mf-ops__filter-item--action">
            <el-button type="primary" size="small" :loading="busy" @click="onSearch">
              搜索
            </el-button>
          </div>
        </div>

        <div v-if="recentKeywords.length" class="mf-ops__recent">
          <span class="mf-ops__recent-label">最近搜索</span>
          <div class="mf-ops__recent-tags">
            <el-tag
              v-for="kw in filteredRecent"
              :key="kw"
              size="small"
              effect="plain"
              :type="query.trim() === kw ? 'primary' : 'info'"
              class="mf-ops__recent-tag"
              closable
              @click="pickRecent(kw)"
              @close="onRemoveRecent(kw)"
            >
              {{ kw }}
            </el-tag>
          </div>
          <el-button link type="info" size="small" @click="onClearRecent">清空</el-button>
        </div>
      </div>
    </section>

    <section class="mf-ops__panel mf-ops__discover-results">
      <div class="mf-ops__panel-head">
        <strong>搜索结果</strong>
        <span
          class="mf-ops__result-meta"
          :class="{
            'is-analyzing':
              phase === 'fetching' || phase === 'revealing' || pageFetching,
            'is-done': phase === 'done' && allItems.length > 0 && !pageFetching,
          }"
        >
          {{ resultMeta }}
        </span>
      </div>
      <div ref="resultsBodyEl" class="mf-ops__panel-body discover-results-body">
        <div v-if="showGrid" class="mf-ops__note-grid">
          <div
            v-for="item in pageItems"
            :key="item.id"
            class="discover-reveal"
            :class="{ 'is-instant': phase === 'done' }"
          >
            <ItemCard :item="item" />
          </div>
          <article
            v-for="n in pendingSkeleton"
            :key="`sk-${n}-${visibleCount}-${pageFetching}`"
            class="note-card note-card--skeleton"
            :style="{ '--sk-i': n - 1 }"
            aria-hidden="true"
          >
            <div class="note-card__cover sk-block">
              <div class="sk-status">
                <span class="sk-pulse" />
                <span class="sk-status__text">{{ skeletonLabel }}</span>
                <span class="sk-status__sub">{{ skeletonSub }}</span>
              </div>
              <div class="sk-progress" aria-hidden="true">
                <span class="sk-progress__bar" />
              </div>
            </div>
            <div class="note-card__body">
              <div class="sk-line sk-line--title" />
              <div class="sk-line sk-line--title sk-line--short" />
              <div class="sk-line sk-line--meta" />
              <div class="sk-actions">
                <span class="sk-btn" />
                <span class="sk-btn" />
                <span class="sk-btn" />
              </div>
            </div>
          </article>
        </div>
        <div v-else class="mf-ops__empty mf-ops__empty--compact">
          <strong>{{ phase === 'error' ? '搜索失败' : '暂无结果' }}</strong>
          <p>
            {{
              phase === 'error'
                ? '请检查账号与网络后重试。'
                : '选择账号并输入关键词后，点击「搜索」。'
            }}
          </p>
        </div>
      </div>
      <div v-if="showPager" class="discover-pager" :class="{ 'is-loading': pageFetching }">
        <el-pagination
          background
          layout="prev, pager, next"
          :total="pagerTotal"
          :page-size="PAGE_SIZE"
          :current-page="currentPage"
          :disabled="pageFetching"
          @current-change="onPageChange"
        />
      </div>
    </section>
  </div>
</template>

<style scoped lang="scss">
.discover {
  display: flex;
  flex-direction: column;
  gap: 12px;
  height: 100%;
  min-height: 0;
  min-width: 0;
  width: 100%;
  max-width: 100%;
  overflow: hidden;
  box-sizing: border-box;
}

.discover :deep(.mf-ops__discover) {
  flex: 0 0 auto;
  min-width: 0;
  width: 100%;
}

.discover :deep(.mf-ops__discover-results) {
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  width: 100%;
  display: flex;
  flex-direction: column;
}

.discover-results-body {
  flex: 1 1 auto;
  min-height: 0;
}

.discover-pager {
  flex: 0 0 auto;
  display: flex;
  justify-content: flex-end;
  align-items: center;
  padding: 10px 14px 12px;
  border-top: 1px solid var(--el-border-color-lighter, #ebeef5);
  background: #fff;

  &.is-loading {
    opacity: 0.65;
  }
}

.mf-ops__result-meta.is-analyzing {
  color: var(--el-color-primary);
}

.mf-ops__result-meta.is-done {
  color: var(--el-color-success);
}

.discover-reveal {
  animation: discover-fade-in 0.45s ease both;
  min-width: 0;
  height: 100%;
}

.discover-reveal.is-instant {
  animation: none;
}

.discover-reveal :deep(.note-card) {
  height: 100%;
}

@keyframes discover-fade-in {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
  to {
    opacity: 1;
    transform: none;
  }
}

.note-card--skeleton {
  pointer-events: none;
  --sk-i: 0;
}

.sk-block,
.sk-line,
.sk-btn {
  background: linear-gradient(90deg, #eef1f6 0%, #f7f8fb 45%, #eef1f6 90%);
  background-size: 200% 100%;
  animation: sk-shimmer 1.2s ease-in-out infinite;
  animation-delay: calc(var(--sk-i) * 0.12s);
}

.note-card--skeleton .note-card__cover.sk-block {
  position: relative;
  width: 100%;
  aspect-ratio: 3 / 4;
  border-radius: 0;
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
}

.sk-status {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 12px 14px;
  max-width: 88%;
  border-radius: 10px;
  background: rgba(15, 23, 42, 0.42);
  backdrop-filter: blur(4px);
  color: #fff;
  text-align: center;
  animation: sk-status-in 0.4s ease both;
  animation-delay: calc(var(--sk-i) * 0.1s);
}

.sk-pulse {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--el-color-primary, #409eff);
  box-shadow: 0 0 0 0 rgba(64, 158, 255, 0.55);
  animation: sk-pulse 1.4s ease-out infinite;
  animation-delay: calc(var(--sk-i) * 0.15s);
}

.sk-status__text {
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.02em;
  line-height: 1.3;
}

.sk-status__sub {
  font-size: 11px;
  line-height: 1.2;
  color: rgba(255, 255, 255, 0.78);
  min-height: 1.2em;
  transition: opacity 0.25s ease;
}

.sk-progress {
  position: absolute;
  left: 10%;
  right: 10%;
  bottom: 14%;
  z-index: 1;
  height: 2px;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.22);
  overflow: hidden;
}

.sk-progress__bar {
  display: block;
  height: 100%;
  width: 40%;
  border-radius: inherit;
  background: linear-gradient(
    90deg,
    transparent,
    var(--el-color-primary-light-3, #79bbff),
    var(--el-color-primary, #409eff),
    transparent
  );
  animation: sk-progress 1.35s ease-in-out infinite;
  animation-delay: calc(var(--sk-i) * 0.12s);
}

.sk-line {
  height: 12px;
  border-radius: 4px;
  margin-bottom: 8px;
}

.sk-line--title {
  height: 14px;
  width: 92%;
}

.sk-line--short {
  width: 64%;
}

.sk-line--meta {
  width: 78%;
  height: 10px;
  margin-top: 4px;
  margin-bottom: 12px;
}

.sk-actions {
  display: flex;
  gap: 6px;
}

.sk-btn {
  flex: 1;
  height: 28px;
  border-radius: 6px;
}

@keyframes sk-shimmer {
  0% {
    background-position: 100% 0;
  }
  100% {
    background-position: -100% 0;
  }
}

@keyframes sk-pulse {
  0% {
    box-shadow: 0 0 0 0 rgba(64, 158, 255, 0.55);
    transform: scale(1);
  }
  70% {
    box-shadow: 0 0 0 8px rgba(64, 158, 255, 0);
    transform: scale(1.05);
  }
  100% {
    box-shadow: 0 0 0 0 rgba(64, 158, 255, 0);
    transform: scale(1);
  }
}

@keyframes sk-progress {
  0% {
    transform: translateX(-120%);
  }
  100% {
    transform: translateX(280%);
  }
}

@keyframes sk-status-in {
  from {
    opacity: 0;
    transform: translateY(6px) scale(0.98);
  }
  to {
    opacity: 1;
    transform: none;
  }
}

@media (prefers-reduced-motion: reduce) {
  .sk-block,
  .sk-line,
  .sk-btn,
  .sk-pulse,
  .sk-progress__bar,
  .sk-status {
    animation: none !important;
  }

  .sk-status {
    opacity: 1;
    transform: none;
  }

  .sk-progress__bar {
    width: 55%;
    transform: none;
    margin: 0 auto;
  }
}
</style>
