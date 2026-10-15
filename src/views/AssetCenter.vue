<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import ImageCraftPanel from '@/components/ImageCraftPanel.vue'
import {
  assetDelete,
  assetDirInfo,
  assetList,
  assetMove,
  assetRename,
  assetReorder,
  assetReveal,
  formatBytes,
  mediaDisplaySrc,
  mediaFolderCreate,
  mediaFolderDelete,
  mediaFolderGet,
  mediaFolderList,
  mediaFolderMove,
  mediaFolderRename,
  mediaFolderReorder,
  type MediaAsset,
  type MediaFolder,
} from '@/api/assets'
import { imageJobDelete, jobStatusLabel, type ImageJob } from '@/api/imageJobs'
import { useImageJobsStore } from '@/state/imageJobs'

const loading = ref(false)
const kind = ref<'all' | 'image' | 'video'>('all')
const query = ref('')
const items = ref<MediaAsset[]>([])
const folders = ref<MediaFolder[]>([])
const dirRoot = ref('')
const preview = ref<MediaAsset | null>(null)
const introOpen = ref(false)
const currentFolderId = ref('')
const crumbs = ref<{ id: string; name: string }[]>([{ id: '', name: '全部素材' }])
const selectedIds = ref<string[]>([])
const selectedFolderIds = ref<string[]>([])
const lastAnchorId = ref('')
const lastFolderAnchorId = ref('')

const craftBaseUrl = ref('')
const craftBaseName = ref('')
const craftBaseDropActive = ref(false)

/** 桌面端 Pointer 拖拽（WKWebView HTML5 drop 常失效） */
const assetDragging = ref(false)
const dragAssetId = ref('')
const dragAssetIds = ref<string[]>([])
const dragFolderId = ref('')
const dragFolderIds = ref<string[]>([])
const dropFolderId = ref<string | null>(null)
const dropSortId = ref<string | null>(null)
const dropInsertBefore = ref(true)
const dragGhost = ref<{
  x: number
  y: number
  url: string
  name: string
  kind: 'asset' | 'folder'
  count: number
} | null>(null)
type DragPress =
  | {
      pointerId: number
      startX: number
      startY: number
      kind: 'asset'
      asset: MediaAsset
      dragging: boolean
      metaKey: boolean
      ctrlKey: boolean
      shiftKey: boolean
    }
  | {
      pointerId: number
      startX: number
      startY: number
      kind: 'folder'
      folder: MediaFolder
      dragging: boolean
      metaKey: boolean
      ctrlKey: boolean
      shiftKey: boolean
    }
let dragPress: DragPress | null = null

const selectedCount = computed(() => selectedIds.value.length + selectedFolderIds.value.length)
const selectedSet = computed(() => new Set(selectedIds.value))
const selectedFolderSet = computed(() => new Set(selectedFolderIds.value))

const imageJobsStore = useImageJobsStore()
const folderJobs = computed(() =>
  imageJobsStore.jobs.value.filter((j) => (j.folderId || '') === currentFolderId.value),
)
const activeJobs = computed(() =>
  folderJobs.value.filter((j) => j.status === 'queued' || j.status === 'running'),
)
const errorJobs = computed(() => folderJobs.value.filter((j) => j.status === 'error'))

type MenuState =
  | { open: false }
  | {
      open: true
      x: number
      y: number
      kind: 'blank' | 'folder' | 'asset' | 'job'
      folder?: MediaFolder
      asset?: MediaAsset
      job?: ImageJob
    }
const menu = ref<MenuState>({ open: false })

const filteredHint = computed(() => {
  if (kind.value === 'image') return '图片'
  if (kind.value === 'video') return '视频'
  return '素材'
})

function sourceLabel(s: string) {
  if (s === 'ai-generate') return 'AI 文生图'
  if (s === 'ai-edit') return 'AI 图生图'
  if (s === 'import') return '导入'
  if (s === 'upload') return '上传'
  return s || '—'
}

function fmtTime(raw?: string) {
  return (raw || '').replace('T', ' ').slice(0, 19) || '—'
}

async function buildCrumbs(folderId: string) {
  const chain: { id: string; name: string }[] = []
  let id = folderId
  while (id) {
    const row = await mediaFolderGet(id)
    if (!row) break
    chain.unshift({ id: row.id, name: row.name })
    id = row.parentId || ''
  }
  crumbs.value = [{ id: '', name: '全部素材' }, ...chain]
}

async function refreshAssets() {
  items.value = await assetList({
    kind: kind.value,
    query: query.value.trim() || undefined,
    folderId: currentFolderId.value,
    limit: 200,
  })
  const alive = new Set(items.value.map((x) => x.id))
  selectedIds.value = selectedIds.value.filter((id) => alive.has(id))
  if (lastAnchorId.value && !alive.has(lastAnchorId.value)) {
    lastAnchorId.value = selectedIds.value[selectedIds.value.length - 1] || ''
  }
}

async function refreshFolders() {
  folders.value = await mediaFolderList(
    currentFolderId.value,
    query.value.trim() || undefined,
  )
  const alive = new Set(folders.value.map((x) => x.id))
  selectedFolderIds.value = selectedFolderIds.value.filter((id) => alive.has(id))
  if (lastFolderAnchorId.value && !alive.has(lastFolderAnchorId.value)) {
    lastFolderAnchorId.value =
      selectedFolderIds.value[selectedFolderIds.value.length - 1] || ''
  }
}

async function onSearch() {
  await Promise.all([refreshFolders(), refreshAssets()])
}

async function refreshJobs() {
  await imageJobsStore.refresh({ folderId: currentFolderId.value })
}

async function refresh() {
  loading.value = true
  try {
    const info = await assetDirInfo()
    dirRoot.value = info.root
    await Promise.all([refreshFolders(), refreshAssets(), refreshJobs(), buildCrumbs(currentFolderId.value)])
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
  }
}

function clearSelection() {
  selectedIds.value = []
  selectedFolderIds.value = []
  lastAnchorId.value = ''
  lastFolderAnchorId.value = ''
}

function isSelected(id: string) {
  return selectedSet.value.has(id)
}

function isFolderSelected(id: string) {
  return selectedFolderSet.value.has(id)
}

async function openFolder(id: string) {
  currentFolderId.value = id || ''
  preview.value = null
  introOpen.value = false
  clearSelection()
  closeMenu()
  await refresh()
}

async function onCreateFolder() {
  closeMenu()
  let name = ''
  try {
    const { value } = await ElMessageBox.prompt('文件夹名称', '新建文件夹', {
      inputPlaceholder: '例如：活动海报',
      confirmButtonText: '创建',
      cancelButtonText: '取消',
      inputValidator: (v) => (String(v || '').trim() ? true : '请输入名称'),
    })
    name = String(value || '').trim()
  } catch {
    return
  }
  try {
    await mediaFolderCreate(name, currentFolderId.value || '')
    ElMessage.success('已创建')
    await refreshFolders()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function onRenameFolder(folder: MediaFolder) {
  closeMenu()
  let name = folder.name
  try {
    const { value } = await ElMessageBox.prompt('新名称', '重命名文件夹', {
      inputValue: folder.name,
      confirmButtonText: '保存',
      cancelButtonText: '取消',
      inputValidator: (v) => (String(v || '').trim() ? true : '请输入名称'),
    })
    name = String(value || '').trim()
  } catch {
    return
  }
  try {
    await mediaFolderRename(folder.id, name)
    ElMessage.success('已重命名')
    await Promise.all([refreshFolders(), buildCrumbs(currentFolderId.value)])
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function onDeleteFolder(folder: MediaFolder) {
  closeMenu()
  const ids =
    isFolderSelected(folder.id) && selectedFolderIds.value.length > 1
      ? [...selectedFolderIds.value]
      : [folder.id]
  try {
    await ElMessageBox.confirm(
      ids.length > 1
        ? `删除选中的 ${ids.length} 个文件夹？内部素材会移回根目录，子文件夹也会删除。`
        : `删除文件夹「${folder.name}」？内部素材会移回根目录，子文件夹也会删除。`,
      '确认',
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    for (const id of ids) {
      await mediaFolderDelete(id)
    }
    clearSelection()
    ElMessage.success(ids.length > 1 ? `已删除 ${ids.length} 个文件夹` : '已删除')
    await refresh()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function onDelete(row: MediaAsset) {
  closeMenu()
  const ids =
    isSelected(row.id) && selectedIds.value.length > 1 ? [...selectedIds.value] : [row.id]
  try {
    await ElMessageBox.confirm(
      ids.length > 1
        ? `删除选中的 ${ids.length} 个素材？将同时删除本地文件。`
        : '删除该素材？将同时删除本地文件。',
      '确认',
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    for (const id of ids) {
      await assetDelete(id)
    }
    if (preview.value && ids.includes(preview.value.id)) {
      preview.value = null
      introOpen.value = false
    }
    clearSelection()
    ElMessage.success(ids.length > 1 ? `已删除 ${ids.length} 项` : '已删除')
    await refreshAssets()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function onReveal(row: MediaAsset) {
  closeMenu()
  try {
    await assetReveal(row.id)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function onRenameAsset(row: MediaAsset) {
  closeMenu()
  let name = row.fileName || ''
  try {
    const { value } = await ElMessageBox.prompt('新文件名（可不写扩展名，将自动保留）', '重命名', {
      inputValue: row.fileName || '',
      confirmButtonText: '保存',
      cancelButtonText: '取消',
      inputValidator: (v) => (String(v || '').trim() ? true : '请输入名称'),
    })
    name = String(value || '').trim()
  } catch {
    return
  }
  try {
    const updated = await assetRename(row.id, name)
    ElMessage.success('已重命名')
    if (preview.value?.id === row.id) preview.value = updated
    await refreshAssets()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

function setAsCraftBase(row: MediaAsset) {
  if (row.kind !== 'image') {
    ElMessage.warning('仅图片可用作底图')
    return
  }
  // bump refs so ImageCraftPanel watch always fires
  craftBaseUrl.value = ''
  craftBaseName.value = ''
  requestAnimationFrame(() => {
    craftBaseUrl.value = row.absPath
    craftBaseName.value = row.fileName || row.prompt || '底图'
  })
  ElMessage.success('已设为图生图底图')
}

function onSelectAsset(
  row: MediaAsset,
  mods?: { metaKey?: boolean; ctrlKey?: boolean; shiftKey?: boolean },
) {
  clearNativeSelection()
  selectedFolderIds.value = []
  lastFolderAnchorId.value = ''
  const ids = items.value.map((x) => x.id)
  const meta = Boolean(mods?.metaKey || mods?.ctrlKey)
  const shift = Boolean(mods?.shiftKey)
  if (shift && lastAnchorId.value) {
    const a = ids.indexOf(lastAnchorId.value)
    const b = ids.indexOf(row.id)
    if (a >= 0 && b >= 0) {
      const lo = Math.min(a, b)
      const hi = Math.max(a, b)
      selectedIds.value = ids.slice(lo, hi + 1)
    } else {
      selectedIds.value = [row.id]
      lastAnchorId.value = row.id
    }
  } else if (meta) {
    if (isSelected(row.id)) {
      selectedIds.value = selectedIds.value.filter((id) => id !== row.id)
    } else {
      selectedIds.value = [...selectedIds.value, row.id]
    }
    lastAnchorId.value = row.id
  } else {
    selectedIds.value = [row.id]
    lastAnchorId.value = row.id
  }
  preview.value = row
}

function onSelectFolder(
  folder: MediaFolder,
  mods?: { metaKey?: boolean; ctrlKey?: boolean; shiftKey?: boolean },
) {
  clearNativeSelection()
  selectedIds.value = []
  lastAnchorId.value = ''
  preview.value = null
  const ids = folders.value.map((x) => x.id)
  const meta = Boolean(mods?.metaKey || mods?.ctrlKey)
  const shift = Boolean(mods?.shiftKey)
  if (shift && lastFolderAnchorId.value) {
    const a = ids.indexOf(lastFolderAnchorId.value)
    const b = ids.indexOf(folder.id)
    if (a >= 0 && b >= 0) {
      const lo = Math.min(a, b)
      const hi = Math.max(a, b)
      selectedFolderIds.value = ids.slice(lo, hi + 1)
    } else {
      selectedFolderIds.value = [folder.id]
      lastFolderAnchorId.value = folder.id
    }
  } else if (meta) {
    if (isFolderSelected(folder.id)) {
      selectedFolderIds.value = selectedFolderIds.value.filter((id) => id !== folder.id)
    } else {
      selectedFolderIds.value = [...selectedFolderIds.value, folder.id]
    }
    lastFolderAnchorId.value = folder.id
  } else {
    selectedFolderIds.value = [folder.id]
    lastFolderAnchorId.value = folder.id
  }
}

function clearNativeSelection() {
  try {
    window.getSelection()?.removeAllRanges()
  } catch {
    /* ignore */
  }
}

function selectAllAssets() {
  clearNativeSelection()
  selectedIds.value = items.value.map((x) => x.id)
  selectedFolderIds.value = folders.value.map((x) => x.id)
  if (items.value.length) {
    lastAnchorId.value = items.value[items.value.length - 1]!.id
    preview.value = items.value[items.value.length - 1]!
  } else {
    lastAnchorId.value = ''
    preview.value = null
  }
  lastFolderAnchorId.value = folders.value[folders.value.length - 1]?.id || ''
}

function onBlankPointerDown(ev: PointerEvent) {
  if (ev.button !== 0) return
  const t = ev.target as HTMLElement | null
  if (!t) return
  if (t.closest('[data-asset-id], [data-grid-folder-id], .assets__job, .assets__crumbs')) return
  clearSelection()
}

function onAssetsKeydown(ev: KeyboardEvent) {
  if ((ev.metaKey || ev.ctrlKey) && ev.key.toLowerCase() === 'a') {
    const el = ev.target as HTMLElement | null
    const tag = el?.tagName
    if (tag === 'INPUT' || tag === 'TEXTAREA' || el?.isContentEditable) return
    ev.preventDefault()
    ev.stopPropagation()
    selectAllAssets()
  } else if (ev.key === 'Escape') {
    clearNativeSelection()
    clearSelection()
  }
}

function onAssetsSelectStart(ev: Event) {
  const el = ev.target as HTMLElement | null
  if (!el) return
  const tag = el.tagName
  if (tag === 'INPUT' || tag === 'TEXTAREA' || el.isContentEditable) return
  if (el.closest('.el-input, .el-textarea, .assets__detail, .assets__path')) return
  ev.preventDefault()
}

function applyReorderMulti(
  list: MediaAsset[],
  movingIds: string[],
  toId: string,
  insertBefore: boolean,
): MediaAsset[] | null {
  const set = new Set(movingIds)
  if (set.has(toId)) return null
  const moving = list.filter((x) => set.has(x.id))
  const rest = list.filter((x) => !set.has(x.id))
  if (!moving.length) return null
  let insertAt = rest.findIndex((x) => x.id === toId)
  if (insertAt < 0) return null
  if (!insertBefore) insertAt += 1
  return [...rest.slice(0, insertAt), ...moving, ...rest.slice(insertAt)]
}

function findDropTarget(
  x: number,
  y: number,
  dragKind: 'asset' | 'folder',
  selfFolderId = '',
) {
  const stack = document.elementsFromPoint(x, y)
  let moveFolderId: string | null = null
  let sortId: string | null = null
  let insertBefore = true
  let overBase = false
  let sortEl: HTMLElement | null = null
  let gridFolderEl: HTMLElement | null = null

  for (const el of stack) {
    if (!(el instanceof HTMLElement)) continue
    if (!overBase && el.closest('[data-craft-base-drop]')) overBase = true

    // 面包屑：移入目标（含根目录空 id）
    if (moveFolderId === null) {
      const crumb = el.closest('[data-crumb-folder-id]') as HTMLElement | null
      if (crumb?.hasAttribute('data-crumb-folder-id')) {
        moveFolderId = crumb.getAttribute('data-crumb-folder-id') ?? ''
      }
    }

    if (!gridFolderEl) {
      const f = el.closest('[data-grid-folder-id]') as HTMLElement | null
      if (f?.dataset.gridFolderId) gridFolderEl = f
    }

    if (!sortEl) {
      if (dragKind === 'asset') {
        const a = el.closest('[data-asset-id]') as HTMLElement | null
        if (a?.dataset.assetId) sortEl = a
      }
    }
  }

  if (dragKind === 'asset') {
    if (moveFolderId === null && gridFolderEl?.dataset.gridFolderId) {
      moveFolderId = gridFolderEl.dataset.gridFolderId
    }
    if (sortEl?.dataset.assetId) {
      sortId = sortEl.dataset.assetId
      const rect = sortEl.getBoundingClientRect()
      insertBefore = x < rect.left + rect.width / 2
    }
    if (moveFolderId !== null && gridFolderEl) {
      sortId = null
    }
  } else if (gridFolderEl?.dataset.gridFolderId) {
    const tid = gridFolderEl.dataset.gridFolderId
    if (tid && tid !== selfFolderId) {
      const rect = gridFolderEl.getBoundingClientRect()
      const edge = Math.max(14, rect.width * 0.22)
      if (x < rect.left + edge) {
        sortId = tid
        insertBefore = true
      } else if (x > rect.right - edge) {
        sortId = tid
        insertBefore = false
      } else if (moveFolderId === null) {
        // 中心区域：移入该文件夹
        moveFolderId = tid
      }
    }
  }

  return { moveFolderId, sortId, insertBefore, overBase }
}

function applyReorder<T extends { id: string }>(
  list: T[],
  fromId: string,
  toId: string,
  insertBefore: boolean,
): T[] | null {
  const ids = list.map((x) => x.id)
  const from = ids.indexOf(fromId)
  if (from < 0 || fromId === toId) return null
  const nextIds = [...ids]
  nextIds.splice(from, 1)
  let insertAt = nextIds.indexOf(toId)
  if (insertAt < 0) return null
  if (!insertBefore) insertAt += 1
  nextIds.splice(insertAt, 0, fromId)
  return nextIds.map((id) => list.find((x) => x.id === id)!).filter(Boolean)
}

function endDragPress() {
  if (!dragPress) return
  window.removeEventListener('pointermove', onDragPointerMove)
  window.removeEventListener('pointerup', onDragPointerUp)
  window.removeEventListener('pointercancel', onDragPointerUp)
  dragPress = null
  assetDragging.value = false
  dragAssetId.value = ''
  dragAssetIds.value = []
  dragFolderId.value = ''
  dragFolderIds.value = []
  dragGhost.value = null
  dropFolderId.value = null
  dropSortId.value = null
  dropInsertBefore.value = true
  craftBaseDropActive.value = false
}

function readMods(ev: PointerEvent | MouseEvent | KeyboardEvent) {
  return {
    metaKey: Boolean(ev.metaKey || (ev as PointerEvent).getModifierState?.('Meta')),
    ctrlKey: Boolean(ev.ctrlKey || (ev as PointerEvent).getModifierState?.('Control')),
    shiftKey: Boolean(ev.shiftKey || (ev as PointerEvent).getModifierState?.('Shift')),
  }
}

function onAssetPointerDown(ev: PointerEvent, row: MediaAsset) {
  if (ev.button !== 0) return
  closeMenu()
  const mods = readMods(ev)
  const multi = mods.metaKey || mods.ctrlKey || mods.shiftKey
  // 多选只在 pointerup 处理一次，避免 down+up 各 toggle 一次互相抵消
  // 普通按下未选中项时先单选，便于直接拖拽
  if (!multi && !isSelected(row.id)) {
    selectedFolderIds.value = []
    lastFolderAnchorId.value = ''
    selectedIds.value = [row.id]
    lastAnchorId.value = row.id
    preview.value = row
  }
  dragPress = {
    pointerId: ev.pointerId,
    startX: ev.clientX,
    startY: ev.clientY,
    kind: 'asset',
    asset: row,
    dragging: false,
    ...mods,
  }
  window.addEventListener('pointermove', onDragPointerMove)
  window.addEventListener('pointerup', onDragPointerUp)
  window.addEventListener('pointercancel', onDragPointerUp)
}

function onFolderPointerDown(ev: PointerEvent, folder: MediaFolder) {
  if (ev.button !== 0) return
  closeMenu()
  const mods = readMods(ev)
  const multi = mods.metaKey || mods.ctrlKey || mods.shiftKey
  if (!multi && !isFolderSelected(folder.id)) {
    selectedIds.value = []
    lastAnchorId.value = ''
    preview.value = null
    selectedFolderIds.value = [folder.id]
    lastFolderAnchorId.value = folder.id
  }
  dragPress = {
    pointerId: ev.pointerId,
    startX: ev.clientX,
    startY: ev.clientY,
    kind: 'folder',
    folder,
    dragging: false,
    ...mods,
  }
  window.addEventListener('pointermove', onDragPointerMove)
  window.addEventListener('pointerup', onDragPointerUp)
  window.addEventListener('pointercancel', onDragPointerUp)
}

function onDragPointerMove(ev: PointerEvent) {
  if (!dragPress || ev.pointerId !== dragPress.pointerId) return
  const dx = ev.clientX - dragPress.startX
  const dy = ev.clientY - dragPress.startY
  if (!dragPress.dragging) {
    if (dx * dx + dy * dy < 36) return
    dragPress.dragging = true
    assetDragging.value = true
    if (dragPress.kind === 'asset') {
      const ids = isSelected(dragPress.asset.id)
        ? items.value.filter((x) => isSelected(x.id)).map((x) => x.id)
        : [dragPress.asset.id]
      dragAssetId.value = dragPress.asset.id
      dragAssetIds.value = ids
      dragGhost.value = {
        x: ev.clientX,
        y: ev.clientY,
        url: dragPress.asset.absPath,
        name: dragPress.asset.fileName || dragPress.asset.prompt || '素材',
        kind: 'asset',
        count: ids.length,
      }
    } else {
      const ids = isFolderSelected(dragPress.folder.id)
        ? folders.value.filter((x) => isFolderSelected(x.id)).map((x) => x.id)
        : [dragPress.folder.id]
      dragFolderId.value = dragPress.folder.id
      dragFolderIds.value = ids
      dragGhost.value = {
        x: ev.clientX,
        y: ev.clientY,
        url: '',
        name: dragPress.folder.name,
        kind: 'folder',
        count: ids.length,
      }
    }
  } else if (dragGhost.value) {
    dragGhost.value = { ...dragGhost.value, x: ev.clientX, y: ev.clientY }
  }
  if (!dragPress.dragging) return
  const selfFid = dragPress.kind === 'folder' ? dragPress.folder.id : ''
  const t = findDropTarget(ev.clientX, ev.clientY, dragPress.kind, selfFid)
  const moving =
    dragPress.kind === 'asset'
      ? dragAssetIds.value.length
        ? dragAssetIds.value
        : [dragPress.asset.id]
      : dragFolderIds.value.length
        ? dragFolderIds.value
        : [dragPress.folder.id]
  dropFolderId.value = t.moveFolderId
  dropSortId.value = t.sortId && !moving.includes(t.sortId) ? t.sortId : null
  dropInsertBefore.value = t.insertBefore
  craftBaseDropActive.value =
    dragPress.kind === 'asset' &&
    t.overBase &&
    dragPress.asset.kind === 'image' &&
    moving.length === 1
}

async function onDragPointerUp(ev: PointerEvent) {
  if (!dragPress || ev.pointerId !== dragPress.pointerId) return
  const press = dragPress
  const selfFid = press.kind === 'folder' ? press.folder.id : ''
  const t = findDropTarget(ev.clientX, ev.clientY, press.kind, selfFid)
  const movingIds =
    press.kind === 'asset'
      ? isSelected(press.asset.id)
        ? items.value.filter((x) => isSelected(x.id)).map((x) => x.id)
        : [press.asset.id]
      : []
  const clickMods = {
    metaKey: press.metaKey,
    ctrlKey: press.ctrlKey,
    shiftKey: press.shiftKey,
  }
  endDragPress()
  if (!press.dragging) {
    // 单击 / ⌘多选 / Shift 连选：只在松手时处理一次
    if (press.kind === 'asset') {
      onSelectAsset(press.asset, clickMods)
    } else {
      onSelectFolder(press.folder, clickMods)
    }
    return
  }
  ev.preventDefault()

  if (press.kind === 'asset') {
    const row = press.asset
    const ids = movingIds.length ? movingIds : [row.id]
    if (t.overBase && row.kind === 'image' && ids.length === 1) {
      setAsCraftBase(row)
      return
    }
    if (t.moveFolderId !== null) {
      if (t.moveFolderId === currentFolderId.value) {
        ElMessage.info('已在当前目录')
        return
      }
      try {
        await assetMove(ids, t.moveFolderId)
        ElMessage.success(
          ids.length > 1
            ? `已移动 ${ids.length} 项`
            : t.moveFolderId
              ? '已移入文件夹'
              : '已移回全部素材',
        )
        if (preview.value && ids.includes(preview.value.id)) {
          preview.value = null
          introOpen.value = false
        }
        clearSelection()
        await refreshAssets()
      } catch (e) {
        ElMessage.error(e instanceof Error ? e.message : String(e))
      }
      return
    }
    if (t.sortId && !ids.includes(t.sortId)) {
      const next =
        ids.length > 1
          ? applyReorderMulti(items.value, ids, t.sortId, t.insertBefore)
          : applyReorder(items.value, row.id, t.sortId, t.insertBefore)
      if (!next) return
      items.value = next
      try {
        await assetReorder(next.map((x) => x.id))
        ElMessage.success(ids.length > 1 ? `已调整 ${ids.length} 项排序` : '已调整排序')
      } catch (e) {
        ElMessage.error(e instanceof Error ? e.message : String(e))
        await refreshAssets()
      }
    }
    return
  }

  // 文件夹：中心/面包屑 = 移入；左右边缘 = 同级排序
  const folderIds = isFolderSelected(press.folder.id)
    ? folders.value.filter((x) => isFolderSelected(x.id)).map((x) => x.id)
    : [press.folder.id]
  if (t.moveFolderId !== null) {
    if (folderIds.includes(t.moveFolderId)) return
    try {
      for (const fid of folderIds) {
        await mediaFolderMove(fid, t.moveFolderId)
      }
      ElMessage.success(
        folderIds.length > 1
          ? `已移动 ${folderIds.length} 个文件夹`
          : t.moveFolderId
            ? '已移入文件夹'
            : '已移到全部素材',
      )
      clearSelection()
      await refreshFolders()
    } catch (e) {
      ElMessage.error(e instanceof Error ? e.message : String(e))
    }
    return
  }
  if (t.sortId && !folderIds.includes(t.sortId)) {
    const next =
      folderIds.length > 1
        ? applyReorderMultiFolders(folders.value, folderIds, t.sortId, t.insertBefore)
        : applyReorder(folders.value, press.folder.id, t.sortId, t.insertBefore)
    if (!next) return
    folders.value = next
    try {
      await mediaFolderReorder(next.map((x) => x.id))
      ElMessage.success(
        folderIds.length > 1 ? `已调整 ${folderIds.length} 个文件夹排序` : '已调整文件夹排序',
      )
    } catch (e) {
      ElMessage.error(e instanceof Error ? e.message : String(e))
      await refreshFolders()
    }
  }
}

function applyReorderMultiFolders(
  list: MediaFolder[],
  movingIds: string[],
  toId: string,
  insertBefore: boolean,
): MediaFolder[] | null {
  const set = new Set(movingIds)
  if (set.has(toId)) return null
  const moving = list.filter((x) => set.has(x.id))
  const rest = list.filter((x) => !set.has(x.id))
  if (!moving.length) return null
  let insertAt = rest.findIndex((x) => x.id === toId)
  if (insertAt < 0) return null
  if (!insertBefore) insertAt += 1
  return [...rest.slice(0, insertAt), ...moving, ...rest.slice(insertAt)]
}

function showIntro(row: MediaAsset) {
  closeMenu()
  preview.value = row
  introOpen.value = true
}

function closeIntro() {
  introOpen.value = false
}

function closeMenu() {
  menu.value = { open: false }
}

function clampMenuPos(x: number, y: number, itemCount: number) {
  const pad = 8
  const w = 176
  const h = Math.max(40, itemCount * 36 + 12)
  const maxX = Math.max(pad, window.innerWidth - w - pad)
  const maxY = Math.max(pad, window.innerHeight - h - pad)
  return {
    x: Math.min(Math.max(pad, x), maxX),
    y: Math.min(Math.max(pad, y), maxY),
  }
}

function openBlankMenu(ev: MouseEvent) {
  ev.preventDefault()
  const { x, y } = clampMenuPos(ev.clientX, ev.clientY, 2)
  menu.value = { open: true, x, y, kind: 'blank' }
}

function openFolderMenu(ev: MouseEvent, folder: MediaFolder) {
  ev.preventDefault()
  ev.stopPropagation()
  if (!isFolderSelected(folder.id)) {
    selectedIds.value = []
    lastAnchorId.value = ''
    preview.value = null
    selectedFolderIds.value = [folder.id]
    lastFolderAnchorId.value = folder.id
  }
  const multi = isFolderSelected(folder.id) && selectedFolderIds.value.length > 1
  const { x, y } = clampMenuPos(ev.clientX, ev.clientY, multi ? 2 : 3)
  menu.value = {
    open: true,
    x,
    y,
    kind: 'folder',
    folder,
  }
}

function openAssetMenu(ev: MouseEvent, asset: MediaAsset) {
  ev.preventDefault()
  ev.stopPropagation()
  if (!isSelected(asset.id)) {
    selectedIds.value = [asset.id]
    lastAnchorId.value = asset.id
  }
  preview.value = asset
  const multi = isSelected(asset.id) && selectedIds.value.length > 1
  const { x, y } = clampMenuPos(ev.clientX, ev.clientY, multi ? 2 : 3)
  menu.value = {
    open: true,
    x,
    y,
    kind: 'asset',
    asset,
  }
}

function openJobMenu(ev: MouseEvent, job: ImageJob) {
  ev.preventDefault()
  ev.stopPropagation()
  const { x, y } = clampMenuPos(ev.clientX, ev.clientY, 1)
  menu.value = {
    open: true,
    x,
    y,
    kind: 'job',
    job,
  }
}

async function onDeleteJob(job: ImageJob) {
  closeMenu()
  try {
    await ElMessageBox.confirm(
      job.status === 'error' ? '删除这条失败记录？' : '取消并删除该生成任务？',
      '确认',
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await imageJobDelete(job.id)
    imageJobsStore.removeLocal(job.id)
    ElMessage.success('已删除')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

function onJobStarted() {
  void refreshJobs()
}

let unsubSettled: (() => void) | null = null

watch(kind, () => {
  void refreshAssets()
})

onMounted(async () => {
  await refresh()
  unsubSettled = imageJobsStore.onSettled(() => {
    void refreshAssets()
    void refreshJobs()
  })
  window.addEventListener('click', closeMenu)
  window.addEventListener('scroll', closeMenu, true)
  window.addEventListener('keydown', onAssetsKeydown)
  document.addEventListener('selectstart', onAssetsSelectStart, true)
})

onBeforeUnmount(() => {
  endDragPress()
  unsubSettled?.()
  window.removeEventListener('click', closeMenu)
  window.removeEventListener('scroll', closeMenu, true)
  window.removeEventListener('keydown', onAssetsKeydown)
  document.removeEventListener('selectstart', onAssetsSelectStart, true)
})
</script>

<template>
  <section
    class="assets"
    :class="{ 'is-asset-dragging': assetDragging }"
    v-loading="loading"
  >
    <div
      v-if="dragGhost"
      class="assets__drag-ghost"
      :class="{ 'is-folder': dragGhost.kind === 'folder' }"
      :style="{ left: `${dragGhost.x}px`, top: `${dragGhost.y}px` }"
    >
      <img
        v-if="dragGhost.kind === 'asset'"
        :src="mediaDisplaySrc(dragGhost.url)"
        alt=""
        draggable="false"
      />
      <span v-else class="assets__drag-ghost-folder" aria-hidden="true" />
      <em v-if="dragGhost.count > 1" class="assets__drag-count">{{ dragGhost.count }}</em>
    </div>
    <header class="assets__head">
      <div>
        <strong>素材中心</strong>
        <p>
          虚拟文件夹管理；可在此直接文生图 / 图生图。存储：
          <code>{{ dirRoot || '…' }}</code>
        </p>
      </div>
    </header>

    <div class="assets__body">
      <aside class="assets__side">
        <div class="assets__craft">
          <h3>AI 生成</h3>
          <ImageCraftPanel
            :folder-id="currentFolderId"
            :external-base-url="craftBaseUrl"
            :external-base-name="craftBaseName"
            :base-drop-active="craftBaseDropActive"
            @started="onJobStarted"
          />
        </div>
      </aside>

      <div class="assets__main" @contextmenu="openBlankMenu">
        <div class="assets__tools">
          <el-select v-model="kind" size="small" class="assets__tool-kind">
            <el-option label="全部" value="all" />
            <el-option label="图片" value="image" />
            <el-option label="视频" value="video" />
          </el-select>
          <el-input
            v-model="query"
            clearable
            size="small"
            class="assets__tool-search"
            placeholder="搜索文件夹 / 文件名 / 提示词"
            @keydown.enter.prevent="onSearch"
            @clear="onSearch"
          />
          <el-button type="primary" size="small" @click="onSearch">搜索</el-button>
          <el-button size="small" @click="onCreateFolder">新建文件夹</el-button>
          <el-button size="small" @click="refresh">刷新</el-button>
        </div>
        <nav class="assets__crumbs">
          <button
            v-for="(c, i) in crumbs"
            :key="`${c.id}-${i}`"
            type="button"
            class="assets__crumb"
            :class="{
              'is-on': i === crumbs.length - 1,
              'is-drop-target': dropFolderId !== null && dropFolderId === c.id,
            }"
            :data-crumb-folder-id="c.id"
            @click="openFolder(c.id)"
          >
            {{ c.name }}
          </button>
          <span v-if="selectedCount" class="assets__sel-hint">已选 {{ selectedCount }} 项</span>
        </nav>
        <div class="assets__scroll" @pointerdown="onBlankPointerDown">
        <div class="assets__grid">
          <button
            v-for="folder in folders"
            :key="folder.id"
            type="button"
            class="assets__item assets__folder"
            :class="{
              'is-on': isFolderSelected(folder.id),
              'is-drop-target': dropFolderId === folder.id,
              'is-drop-before': dropSortId === folder.id && dropInsertBefore,
              'is-drop-after': dropSortId === folder.id && !dropInsertBefore,
              'is-dragging-src':
                dragFolderIds.includes(folder.id) || dragFolderId === folder.id,
            }"
            :data-grid-folder-id="folder.id"
            :title="folder.name"
            @pointerdown.stop="onFolderPointerDown($event, folder)"
            @dblclick="openFolder(folder.id)"
            @contextmenu="openFolderMenu($event, folder)"
          >
            <span class="assets__icon assets__icon--folder" aria-hidden="true" />
            <span class="assets__name">{{ folder.name }}</span>
          </button>

          <div
            v-for="job in activeJobs"
            :key="job.id"
            class="assets__item assets__job"
            :title="job.prompt || jobStatusLabel(job.status)"
            @contextmenu="openJobMenu($event, job)"
          >
            <span class="assets__icon assets__icon--job">
              <strong>{{ jobStatusLabel(job.status) }}</strong>
            </span>
            <span class="assets__name">{{ job.prompt || '生成中…' }}</span>
          </div>

          <div
            v-for="job in errorJobs"
            :key="job.id"
            class="assets__item assets__job is-error"
            :title="job.error || '失败'"
            @contextmenu="openJobMenu($event, job)"
          >
            <span class="assets__icon assets__icon--job">
              <strong>失败</strong>
            </span>
            <span class="assets__name">{{ job.prompt || '任务失败' }}</span>
          </div>

          <button
            v-for="row in items"
            :key="row.id"
            type="button"
            class="assets__item"
            :class="{
              'is-on': isSelected(row.id),
              'is-drop-before': dropSortId === row.id && dropInsertBefore,
              'is-drop-after': dropSortId === row.id && !dropInsertBefore,
              'is-dragging-src': dragAssetIds.includes(row.id) || dragAssetId === row.id,
            }"
            :data-asset-id="row.id"
            :title="`${row.prompt || row.fileName || '未命名'}\n${sourceLabel(row.source)} · ${formatBytes(row.sizeBytes)}`"
            @pointerdown.stop="onAssetPointerDown($event, row)"
            @dblclick="showIntro(row)"
            @contextmenu="openAssetMenu($event, row)"
          >
            <span class="assets__icon assets__icon--media">
              <img
                v-if="row.kind === 'image'"
                :src="mediaDisplaySrc(row.absPath)"
                alt=""
                loading="lazy"
                draggable="false"
              />
              <span v-else class="assets__video-ph">视频</span>
            </span>
            <span class="assets__name">{{ row.fileName || row.prompt || '未命名' }}</span>
          </button>

          <div
            v-if="!folders.length && !items.length && !activeJobs.length && !errorJobs.length"
            class="assets__empty"
          >
            <strong>此目录暂无{{ filteredHint }}</strong>
            <p>单击选中，文件夹双击进入；⌘/Ctrl 多选，Shift 连选。</p>
          </div>
        </div>
        </div>
      </div>
    </div>

    <div
      v-if="menu.open"
      class="assets__menu"
      :style="{ left: `${menu.x}px`, top: `${menu.y}px` }"
      @click.stop
    >
      <template v-if="menu.kind === 'blank'">
        <button type="button" @click="onCreateFolder">新建文件夹</button>
        <button type="button" @click="refresh">刷新</button>
      </template>
      <template v-else-if="menu.kind === 'folder' && menu.folder">
        <button
          v-if="!(isFolderSelected(menu.folder.id) && selectedFolderIds.length > 1)"
          type="button"
          @click="openFolder(menu.folder!.id)"
        >
          打开
        </button>
        <button
          v-if="!(isFolderSelected(menu.folder.id) && selectedFolderIds.length > 1)"
          type="button"
          @click="onRenameFolder(menu.folder!)"
        >
          重命名
        </button>
        <button type="button" class="is-danger" @click="onDeleteFolder(menu.folder!)">
          {{
            isFolderSelected(menu.folder.id) && selectedFolderIds.length > 1
              ? `删除（${selectedFolderIds.length}）`
              : '删除'
          }}
        </button>
      </template>
      <template v-else-if="menu.kind === 'asset' && menu.asset">
        <button
          v-if="!(isSelected(menu.asset.id) && selectedCount > 1)"
          type="button"
          @click="showIntro(menu.asset!)"
        >
          显示简介
        </button>
        <button
          v-if="!(isSelected(menu.asset.id) && selectedCount > 1)"
          type="button"
          @click="onRenameAsset(menu.asset!)"
        >
          重命名
        </button>
        <button
          v-if="!(isSelected(menu.asset.id) && selectedCount > 1)"
          type="button"
          @click="onReveal(menu.asset!)"
        >
          在访达中显示
        </button>
        <button type="button" class="is-danger" @click="onDelete(menu.asset!)">
          {{
            isSelected(menu.asset.id) && selectedCount > 1
              ? `删除（${selectedCount}）`
              : '删除'
          }}
        </button>
      </template>
      <template v-else-if="menu.kind === 'job' && menu.job">
        <button type="button" class="is-danger" @click="onDeleteJob(menu.job!)">删除</button>
      </template>
    </div>

    <el-dialog
      v-model="introOpen"
      title="素材简介"
      width="420px"
      append-to-body
      destroy-on-close
      class="assets-intro-dialog"
      @closed="closeIntro"
    >
      <div v-if="preview" class="assets__detail">
        <div class="assets__preview">
          <img
            v-if="preview.kind === 'image'"
            :src="mediaDisplaySrc(preview.absPath)"
            alt=""
          />
          <video
            v-else
            :src="mediaDisplaySrc(preview.absPath)"
            controls
          />
        </div>
        <dl class="assets__dl">
          <div>
            <dt>类型</dt>
            <dd>{{ preview.kind === 'video' ? '视频' : '图片' }}</dd>
          </div>
          <div>
            <dt>来源</dt>
            <dd>{{ sourceLabel(preview.source) }}</dd>
          </div>
          <div>
            <dt>模型</dt>
            <dd>{{ preview.model || '—' }}</dd>
          </div>
          <div>
            <dt>大小</dt>
            <dd>{{ formatBytes(preview.sizeBytes) }}</dd>
          </div>
          <div>
            <dt>时间</dt>
            <dd>{{ fmtTime(preview.createdAt) }}</dd>
          </div>
          <div>
            <dt>提示词</dt>
            <dd>{{ preview.prompt || '—' }}</dd>
          </div>
          <div>
            <dt>路径</dt>
            <dd class="assets__path">{{ preview.absPath }}</dd>
          </div>
        </dl>
        <div class="assets__actions">
          <el-button size="small" @click="onRenameAsset(preview)">重命名</el-button>
          <el-button size="small" @click="onReveal(preview)">在访达中显示</el-button>
          <el-button size="small" type="danger" plain @click="onDelete(preview)">删除</el-button>
        </div>
      </div>
    </el-dialog>
  </section>
</template>

<style scoped lang="scss">
.assets {
  position: relative;
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 20px 24px;
  gap: 10px;
  background: var(--panel, #fff);
  user-select: none;
  -webkit-user-select: none;

  :deep(input),
  :deep(textarea),
  :deep(.el-input__inner),
  :deep(.el-textarea__inner) {
    user-select: text;
    -webkit-user-select: text;
  }

  &.is-asset-dragging {
    cursor: grabbing;
  }
}
.assets__drag-ghost {
  position: fixed;
  z-index: 2000;
  width: 56px;
  height: 56px;
  margin: -28px 0 0 -28px;
  border-radius: 8px;
  overflow: visible;
  pointer-events: none;
  box-shadow: 0 8px 24px rgba(15, 23, 42, 0.25);
  border: 2px solid #fff;
  background: #fff;
  display: grid;
  place-items: center;
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  &.is-folder {
    background: transparent;
    border: none;
    box-shadow: none;
  }
}
.assets__drag-ghost-folder {
  width: 48px;
  height: 38px;
  border-radius: 3px 6px 6px 6px;
  background: linear-gradient(180deg, #7ec8f5 0%, #4ba3e3 55%, #3b8fd0 100%);
  position: relative;
  &::before {
    content: '';
    position: absolute;
    left: 0;
    top: -6px;
    width: 18px;
    height: 8px;
    border-radius: 3px 4px 0 0;
    background: #4ba3e3;
  }
}
.assets__head {
  display: flex;
  align-items: flex-start;
  gap: 16px;

  strong {
    font-size: 18px;
  }
  p {
    margin: 6px 0 0;
    font-size: 12px;
    color: var(--muted, #888);
    max-width: 720px;
    word-break: break-all;
  }
  code {
    font-size: 11px;
    color: var(--el-color-primary);
  }
}
.assets__tools {
  flex-shrink: 0;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.assets__tool-kind {
  width: 100px;
  flex-shrink: 0;
}
.assets__tool-search {
  flex: 1;
  min-width: 140px;
  max-width: 280px;
}
.assets__crumbs {
  flex-shrink: 0;
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  align-items: center;
  margin-bottom: 10px;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--border, #eef0f3);
  background: var(--panel, #fff);
}
.assets__sel-hint {
  margin-left: auto;
  font-size: 12px;
  color: var(--el-color-primary);
  font-weight: 600;
}
.assets__drag-count {
  position: absolute;
  right: -6px;
  top: -6px;
  min-width: 20px;
  height: 20px;
  padding: 0 5px;
  border-radius: 999px;
  background: var(--el-color-primary);
  color: #fff;
  font-size: 11px;
  font-weight: 700;
  font-style: normal;
  display: grid;
  place-items: center;
  box-shadow: 0 1px 4px rgba(15, 23, 42, 0.25);
}
.assets__crumb {
  border: none;
  background: none;
  padding: 2px 6px;
  border-radius: 6px;
  font-size: 12px;
  color: var(--muted, #888);
  cursor: pointer;

  &:not(:last-child)::after {
    content: '/';
    margin-left: 6px;
    color: #ccc;
  }
  &.is-on {
    color: var(--text, #222);
    font-weight: 600;
    background: #f3f4f6;
  }
  &:hover {
    background: #f3f4f6;
  }
  &.is-drop-target {
    color: var(--el-color-primary);
    background: var(--el-color-primary-light-8, #d9ecff);
  }
}
.assets__body {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(300px, 340px) 1fr;
  gap: 14px;
  overflow: hidden;
}
.assets__main {
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding: 4px 2px 0 14px;
  border-left: 1px solid var(--border, #e5e7eb);
}
.assets__scroll {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding-bottom: 12px;
}
.assets__grid {
  min-height: min-content;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(88px, 1fr));
  gap: 6px 8px;
  align-content: start;
}
.assets__item {
  appearance: none;
  border: none;
  background: transparent;
  width: 100%;
  max-width: 88px;
  margin: 0 auto;
  padding: 4px 2px 4px;
  border-radius: 8px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  cursor: pointer;
  color: inherit;
  font: inherit;

  touch-action: none;

  &:hover {
    background: var(--el-color-primary-light-9, #ecf5ff);
  }
  &.is-on {
    background: var(--el-color-primary-light-8, #d9ecff);
  }
  &.is-drop-target {
    background: var(--el-color-primary-light-7, #c6e2ff);
  }
  &.is-drop-before {
    box-shadow: inset 3px 0 0 var(--el-color-primary);
  }
  &.is-drop-after {
    box-shadow: inset -3px 0 0 var(--el-color-primary);
  }
  &.is-dragging-src {
    opacity: 0.45;
  }
}
.assets__job {
  cursor: default;
  &.is-error .assets__icon--job strong {
    color: #ef4444;
  }
}
.assets__icon {
  width: 56px;
  height: 56px;
  flex-shrink: 0;
  display: grid;
  place-items: center;
}
.assets__icon--folder {
  width: 58px;
  height: 46px;
  margin: 5px 0;
  border-radius: 3px 6px 6px 6px;
  background: linear-gradient(180deg, #7ec8f5 0%, #4ba3e3 55%, #3b8fd0 100%);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.45),
    0 1px 2px rgba(15, 23, 42, 0.12);
  position: relative;

  &::before {
    content: '';
    position: absolute;
    left: 0;
    top: -7px;
    width: 22px;
    height: 10px;
    border-radius: 3px 4px 0 0;
    background: linear-gradient(180deg, #6bb8ec 0%, #4ba3e3 100%);
  }
}
.assets__icon--media {
  border-radius: 8px;
  overflow: hidden;
  background: #f3f4f6;
  border: 1px solid #e8eaee;
  box-shadow: 0 1px 2px rgba(15, 23, 42, 0.06);

  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
}
.assets__icon--job {
  border-radius: 8px;
  background: var(--el-color-primary-light-9, #ecf5ff);
  border: 1px dashed var(--el-color-primary-light-5, #a0cfff);

  strong {
    font-size: 11px;
    font-weight: 700;
    color: var(--el-color-primary);
  }
}
.assets__video-ph {
  font-size: 11px;
  color: var(--muted, #888);
}
.assets__name {
  width: 100%;
  font-size: 12px;
  line-height: 1.3;
  text-align: center;
  overflow: hidden;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  word-break: break-all;
}
.assets__empty {
  grid-column: 1 / -1;
  padding: 48px 16px;
  text-align: center;
  color: var(--muted, #888);
  strong {
    display: block;
    color: var(--text, #222);
    margin-bottom: 6px;
  }
}
.assets__side {
  min-height: 0;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 12px;
  position: sticky;
  top: 0;
  align-self: stretch;
}
.assets__craft {
  border: 1px solid var(--border, #e5e7eb);
  border-radius: 12px;
  padding: 14px;
  background: #fafafa;
  height: 100%;
  min-height: 0;
  overflow: auto;
  box-sizing: border-box;

  h3 {
    margin: 0 0 12px;
    font-size: 15px;
  }
}
.assets__detail {
  padding: 0 4px 4px;
}
.assets__preview {
  border-radius: 10px;
  overflow: hidden;
  background: #111;
  margin-bottom: 12px;
  img,
  video {
    width: 100%;
    display: block;
    max-height: 280px;
    object-fit: contain;
  }
}
.assets__dl {
  margin: 0;
  display: grid;
  gap: 8px;
  font-size: 12px;

  div {
    display: grid;
    grid-template-columns: 56px 1fr;
    gap: 8px;
  }
  dt {
    color: var(--muted, #888);
  }
  dd {
    margin: 0;
    word-break: break-word;
  }
}
.assets__path {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
}
.assets__actions {
  margin-top: 14px;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.assets__menu {
  position: fixed;
  z-index: 1000;
  min-width: 168px;
  max-width: min(240px, calc(100vw - 16px));
  padding: 6px;
  border-radius: 10px;
  border: 1px solid var(--border, #e5e7eb);
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
    white-space: nowrap;
    cursor: pointer;
    &:hover {
      background: #f3f4f6;
    }
    &.is-danger {
      color: #ef4444;
    }
  }
}

@media (max-width: 1100px) {
  .assets__body {
    grid-template-columns: 1fr;
  }
}
</style>
