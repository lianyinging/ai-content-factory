<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { Platform } from '@/types/item'
import {
  draftDelete,
  draftList,
  draftUpsert,
  parseImages,
  parseTags,
  type DraftRow,
} from '@/api/drafts'
import { assetIngest, mediaDisplaySrc } from '@/api/assets'
import {
  productDelete,
  productList,
  productUpsert,
  type ProductItem,
} from '@/api/products'
import { imageJobStart, jobStatusLabel, type ImageJob } from '@/api/imageJobs'
import { useImageJobsStore } from '@/state/imageJobs'
import {
  aiGenerate,
  aiOptimizePrompt,
  aiRewrite,
  aiTags,
  aiTitles,
} from '@/api/ai'

const props = defineProps<{ platform: Platform; label: string }>()
const router = useRouter()

const IMAGE_RATIOS = [
  { label: '1:1', size: '1024x1024' },
  { label: '3:4', size: '1024x1536' },
  { label: '16:9', size: '1536x1024' },
] as const

const IMAGE_QUALITIES = [
  { value: 'low', label: '标准' },
  { value: 'medium', label: '高清' },
  { value: 'high', label: '超清' },
] as const

/** 图生图 · 首饰佩戴预设（点击填入编辑指令） */
const JEWELRY_WEAR_PRESETS = [
  {
    label: '手模特写',
    prompt:
      '将画面中的首饰改为一枚戴在优雅女性手指上的展示图，手部特写，手指纤细白皙，自然手势，柔和棚拍光，浅景深，保留首饰原有材质、颜色与细节，无文字水印，小红书种草风',
  },
  {
    label: '耳部佩戴',
    prompt:
      '将画面中的耳饰改为佩戴在女性侧脸耳部的实拍效果，露出耳廓与少量发丝，轻妆，柔和自然光，背景虚化，保留耳饰原设计与光泽，高清细节，无文字',
  },
  {
    label: '颈间佩戴',
    prompt:
      '将画面中的项链改为佩戴在女性锁骨颈间的特写，肌肤细腻，锁骨线条自然，轻柔侧光，高级感，保留项链造型与质感，浅景深，无文字水印',
  },
  {
    label: '手腕佩戴',
    prompt:
      '将画面中的手链/手镯改为戴在女性纤细手腕上的特写，手势自然放松，肤色干净，柔光棚拍，保留原有材质与细节，背景简洁，无文字',
  },
  {
    label: '对戒/叠戴',
    prompt:
      '将画面中的戒指改为女性双手交叠佩戴的特写，可轻微叠戴感，指尖干净，温柔自然光，保留戒托与宝石原貌，小红书产品种草风，无文字',
  },
  {
    label: '试戴自拍感',
    prompt:
      '将画面中的首饰改为近距离手机自拍视角的试戴效果，真人感但不露完整五官，聚焦佩戴部位，生活感光线，保留首饰真实细节，适合小红书笔记配图，无文字水印',
  },
] as const

type CraftMode = 'generate' | 'edit'

const loading = ref(false)
/** 仅当前操作指示忙碌，避免整页/多按钮同时 loading 导致光标像全局加载 */
type DraftBusy =
  | 'generate'
  | 'rewrite'
  | 'titles'
  | 'tags'
  | 'image'
  | 'publish'
  | 'optimize-copy'
  | 'optimize-image'
const busyAction = ref<DraftBusy | null>(null)
const isBusy = computed(() => busyAction.value !== null)
const titlePickBusy = ref(false)
const drafts = ref<DraftRow[]>([])
const selectedDraftId = ref('')
const draftMode = ref<'rewrite' | 'generate'>('rewrite')
const aiTab = ref<'copy' | 'image'>('copy')

const aiInstruction = ref('')
const generateTopic = ref('')
const generateReference = ref('')
const products = ref<ProductItem[]>([])
const productsLoading = ref(false)
const selectedProductSku = ref('')
const productCategoryFilter = ref('')
const productPage = ref(1)
const PRODUCT_PAGE_SIZE = 8
const productUploadVisible = ref(false)
const productUploading = ref(false)
const productUploadForm = ref({
  sku: '',
  name: '',
  spec: '',
  category: '',
  unit: '',
  imageBase64: '',
  previewUrl: '',
})
const titleCandidates = ref<string[]>([])
const newTagInput = ref('')
const imagePrompt = ref('')
const editPrompt = ref('')
const craftMode = ref<CraftMode>('generate')
const imageSize = ref('1024x1024')
const imageQuality = ref('medium')
const baseImageUrl = ref('')
const baseImageName = ref('')
const baseFileInput = ref<HTMLInputElement | null>(null)
const baseDropActive = ref(false)
const imagesDropActive = ref(false)
const imageJobsStore = useImageJobsStore()
const draftImageJobs = computed(() =>
  selectedDraftId.value
    ? imageJobsStore.jobs.value.filter((j) => j.draftId === selectedDraftId.value)
    : [],
)
const pendingDoneJobs = computed(() =>
  draftImageJobs.value.filter((j) => j.status === 'done' && j.absPath),
)
const activeImageJobs = computed(() =>
  draftImageJobs.value.filter((j) => j.status === 'queued' || j.status === 'running'),
)
/** 桌面端用 Pointer 拖拽（HTML5 drop 在 WKWebView 常失效） */
const draftDragging = ref(false)
type DragSource = 'draft' | 'pending'
const dragGhost = ref<{
  x: number
  y: number
  url: string
  idx: number
  name: string
  source: DragSource
  pendingId?: string
} | null>(null)
type DraftPress = {
  pointerId: number
  startX: number
  startY: number
  url: string
  idx: number
  name: string
  source: DragSource
  pendingId?: string
  dragging: boolean
  overBase: boolean
  overImages: boolean
}
let draftPress: DraftPress | null = null

const title = ref('')
const body = ref('')
const tags = ref<string[]>([])
const images = ref<string[]>([])

const isEditMode = computed(() => craftMode.value === 'edit')
const activeImagePrompt = computed({
  get: () => (isEditMode.value ? editPrompt.value : imagePrompt.value),
  set: (v: string) => {
    if (isEditMode.value) editPrompt.value = v
    else imagePrompt.value = v
  },
})

const selectedDraft = computed(
  () => drafts.value.find((d) => d.id === selectedDraftId.value) || null,
)

const publishReady = computed(() => {
  const hasTitle = Boolean(title.value.trim())
  const hasBody = Boolean(body.value.trim())
  const hasImage = images.value.length > 0
  return hasTitle && hasBody && hasImage
})

const publishHint = computed(() => {
  const miss: string[] = []
  if (!title.value.trim()) miss.push('标题')
  if (!body.value.trim()) miss.push('正文')
  if (!images.value.length) miss.push('配图')
  return miss.length ? `还需完善：${miss.join('、')}` : '内容已齐，可进入发布中心'
})

function formatTime(raw?: string) {
  if (!raw) return '—'
  try {
    const d = new Date(raw.includes('T') ? raw : raw.replace(' ', 'T'))
    if (Number.isNaN(d.getTime())) {
      return raw.replace('T', ' ').slice(0, 16)
    }
    const mm = String(d.getMonth() + 1).padStart(2, '0')
    const dd = String(d.getDate()).padStart(2, '0')
    const hh = String(d.getHours()).padStart(2, '0')
    const mi = String(d.getMinutes()).padStart(2, '0')
    return `${mm}-${dd} ${hh}:${mi}`
  } catch {
    return raw
  }
}

function loadForm(d: DraftRow | null) {
  if (!d) {
    title.value = ''
    body.value = ''
    tags.value = []
    images.value = []
    return
  }
  title.value = d.title
  body.value = d.body
  tags.value = parseTags(d.tagsJson)
  images.value = parseImages(d.imageUrlsJson)
}

async function refresh(opts?: { silent?: boolean }) {
  const silent = Boolean(opts?.silent)
  if (!silent) loading.value = true
  try {
    drafts.value = await draftList(props.platform)
    const qid = String(router.currentRoute.value.query.draftId || '')
    if (qid && drafts.value.some((d) => d.id === qid)) {
      selectedDraftId.value = qid
    } else if (!selectedDraftId.value || !drafts.value.some((d) => d.id === selectedDraftId.value)) {
      selectedDraftId.value = drafts.value[0]?.id || ''
    }
    loadForm(selectedDraft.value)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    if (!silent) loading.value = false
  }
}

async function onNewDraft() {
  try {
    const row = await draftUpsert({
      platform: props.platform,
      title: '未命名草稿',
      body: '',
      tagsJson: '[]',
      imageUrlsJson: '[]',
    })
    draftMode.value = 'rewrite'
    selectedDraftId.value = row.id
    ElMessage.success('已新建')
    await refresh({ silent: true })
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function onSaveDraft(opts?: { quiet?: boolean }) {
  if (!selectedDraftId.value) return
  try {
    await draftUpsert({
      id: selectedDraftId.value,
      platform: props.platform,
      title: title.value,
      body: body.value,
      tagsJson: JSON.stringify(tags.value),
      imageUrlsJson: JSON.stringify(images.value),
    })
    if (!opts?.quiet) ElMessage.success('草稿已保存')
    await refresh({ silent: true })
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function onDeleteDraft(id: string) {
  try {
    await ElMessageBox.confirm('删除该草稿？', '确认', { type: 'warning' })
  } catch {
    return
  }
  try {
    await draftDelete(id)
    if (selectedDraftId.value === id) selectedDraftId.value = ''
    await refresh({ silent: true })
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

function addDraftTag() {
  const t = newTagInput.value.trim().replace(/^#/, '')
  if (!t) return
  if (!tags.value.includes(t)) tags.value = [...tags.value, t]
  newTagInput.value = ''
}

function removeDraftTag(tag: string) {
  tags.value = tags.value.filter((x) => x !== tag)
}

function removeDraftImage(url: string) {
  images.value = images.value.filter((u) => u !== url)
}

async function addDraftImage(url: string, opts?: { quiet?: boolean }) {
  const stored = String(url || '').trim()
  if (!stored) return false
  if (images.value.includes(stored)) {
    if (!opts?.quiet) ElMessage.info('该图已在配图列表中')
    return false
  }
  images.value = [...images.value, stored]
  await onSaveDraft({ quiet: true })
  if (!opts?.quiet) ElMessage.success('已加入配图')
  return true
}

async function useJobImage(job: ImageJob) {
  if (!job.absPath) return
  await addDraftImage(job.absPath)
}

async function hydrateDraftImageJobs() {
  if (!selectedDraftId.value) return
  await imageJobsStore.refresh({ draftId: selectedDraftId.value })
}

const selectedProduct = computed(() =>
  products.value.find((p) => p.sku === selectedProductSku.value) || null,
)

const productCategories = computed(() => {
  const set = new Set<string>()
  for (const p of products.value) {
    const c = (p.category || '').trim()
    if (c) set.add(c)
  }
  return Array.from(set).sort()
})

const filteredProducts = computed(() => {
  const cat = productCategoryFilter.value.trim()
  if (!cat) return products.value
  return products.value.filter((p) => (p.category || '').trim() === cat)
})

const pagedProducts = computed(() => {
  const list = filteredProducts.value
  const start = (productPage.value - 1) * PRODUCT_PAGE_SIZE
  return list.slice(start, start + PRODUCT_PAGE_SIZE)
})

const productPagerVisible = computed(
  () => filteredProducts.value.length > PRODUCT_PAGE_SIZE,
)

watch(productCategoryFilter, () => {
  productPage.value = 1
})

watch(filteredProducts, (list) => {
  const maxPage = Math.max(1, Math.ceil(list.length / PRODUCT_PAGE_SIZE) || 1)
  if (productPage.value > maxPage) productPage.value = maxPage
})

function productReferenceText(p: ProductItem) {
  // 不把货号写入参考，避免 AI 把内部 SKU 写进小红书正文
  const parts = [
    p.category ? `分类：${p.category}` : '',
    p.spec ? `规格：${p.spec}` : '',
    p.unit ? `单位：${p.unit}` : '',
    p.description ? `描述：${p.description}` : '',
  ].filter(Boolean)
  return parts.join('\n')
}

function selectProduct(p: ProductItem) {
  if (selectedProductSku.value === p.sku) {
    selectedProductSku.value = ''
    return
  }
  selectedProductSku.value = p.sku
  generateTopic.value = p.name
  generateReference.value = productReferenceText(p)
}

async function refreshProducts() {
  productsLoading.value = true
  try {
    products.value = await productList()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    productsLoading.value = false
  }
}

function resetProductUploadForm() {
  productUploadForm.value = {
    sku: '',
    name: '',
    spec: '',
    category: '',
    unit: '',
    imageBase64: '',
    previewUrl: '',
  }
}

function onProductFileChange(ev: Event) {
  const input = ev.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = ''
  if (!file) return
  const reader = new FileReader()
  reader.onload = () => {
    const dataUrl = String(reader.result || '')
    productUploadForm.value.imageBase64 = dataUrl
    productUploadForm.value.previewUrl = dataUrl
    if (!productUploadForm.value.name.trim()) {
      productUploadForm.value.name = file.name.replace(/\.[^.]+$/, '')
    }
  }
  reader.readAsDataURL(file)
}

async function submitProductUpload() {
  const f = productUploadForm.value
  if (!f.name.trim()) {
    ElMessage.warning('请填写品名')
    return
  }
  if (!f.imageBase64) {
    ElMessage.warning('请上传产品图')
    return
  }
  productUploading.value = true
  try {
    const row = await productUpsert({
      sku: f.sku.trim() || undefined,
      name: f.name.trim(),
      spec: f.spec.trim() || undefined,
      category: f.category.trim() || undefined,
      unit: f.unit.trim() || undefined,
      imageBase64: f.imageBase64,
    })
    productUploadVisible.value = false
    resetProductUploadForm()
    await refreshProducts()
    selectProduct(row)
    ElMessage.success('已加入产品库')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    productUploading.value = false
  }
}

async function onDeleteProduct(p: ProductItem, ev: Event) {
  ev.stopPropagation()
  if (p.source !== 'user') return
  try {
    await ElMessageBox.confirm(`删除自定义产品「${p.name}」？`, '删除产品', {
      type: 'warning',
      confirmButtonText: '删除',
      cancelButtonText: '取消',
    })
  } catch {
    return
  }
  try {
    await productDelete(p.sku)
    if (selectedProductSku.value === p.sku) selectedProductSku.value = ''
    await refreshProducts()
    ElMessage.success('已删除')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function onGenerateDraft() {
  if (!generateTopic.value.trim()) {
    ElMessage.warning('请输入主题')
    return
  }
  if (isBusy.value) return
  busyAction.value = 'generate'
  try {
    const r = await aiGenerate({
      topic: generateTopic.value.trim(),
      reference: generateReference.value.trim(),
      instruction: aiInstruction.value.trim(),
    })
    const imagePaths: string[] = []
    if (selectedProduct.value?.imageAbsPath) {
      try {
        const asset = await assetIngest({
          sourceUrl: selectedProduct.value.imageAbsPath,
          kind: 'image',
          source: 'product',
          platform: props.platform,
          fileName: `${selectedProduct.value.sku || 'product'}.jpg`,
        })
        if (asset.absPath) imagePaths.push(asset.absPath)
      } catch {
        imagePaths.push(selectedProduct.value.imageAbsPath)
      }
    }
    const row = await draftUpsert({
      platform: props.platform,
      title: r.title || generateTopic.value.slice(0, 20),
      body: r.body,
      tagsJson: JSON.stringify(r.tags || []),
      imageUrlsJson: JSON.stringify(imagePaths),
    })
    draftMode.value = 'rewrite'
    selectedDraftId.value = row.id
    ElMessage.success('已生成草稿')
    await refresh({ silent: true })
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busyAction.value = null
  }
}

async function runDraftAi(kind: 'rewrite' | 'titles' | 'tags') {
  if (!selectedDraft.value) return
  if (isBusy.value) return
  busyAction.value = kind
  try {
    if (kind === 'rewrite') {
      body.value = await aiRewrite({
        title: title.value,
        body: body.value,
        instruction: aiInstruction.value,
      })
      await onSaveDraft({ quiet: true })
      ElMessage.success('已改写并保存')
    } else if (kind === 'titles') {
      titleCandidates.value = await aiTitles({
        title: title.value,
        body: body.value,
        count: 5,
      })
    } else {
      tags.value = await aiTags({
        title: title.value,
        body: body.value,
        existing: tags.value,
      })
      await onSaveDraft({ quiet: true })
      ElMessage.success('已生成标签并保存')
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busyAction.value = null
  }
}

async function applyTitleCandidate(t: string) {
  title.value = t.slice(0, 40)
  titleCandidates.value = []
  titlePickBusy.value = true
  try {
    await onSaveDraft()
    ElMessage.success('已选用标题')
  } finally {
    titlePickBusy.value = false
  }
}

function setCraftMode(mode: CraftMode) {
  craftMode.value = mode
}

function clearBaseImage() {
  baseImageUrl.value = ''
  baseImageName.value = ''
  if (baseFileInput.value) baseFileInput.value.value = ''
}

function pickBaseFromDraft(url: string, idx: number) {
  baseImageUrl.value = url
  baseImageName.value = idx === 0 ? '封面' : `配图 ${idx + 1}`
  craftMode.value = 'edit'
  aiTab.value = 'image'
}

function openBaseFilePicker() {
  baseFileInput.value?.click()
}

function readFileAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result || ''))
    reader.onerror = () => reject(new Error('读取底图失败'))
    reader.readAsDataURL(file)
  })
}

async function onBaseFileChange(ev: Event) {
  const input = ev.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  if (!file.type.startsWith('image/')) {
    ElMessage.warning('请选择图片文件')
    return
  }
  try {
    baseImageUrl.value = await readFileAsDataUrl(file)
    baseImageName.value = file.name || '本地底图'
    craftMode.value = 'edit'
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

function findDropZone(x: number, y: number, selector: string) {
  // 临时隐藏 ghost，避免挡在 elementFromPoint 下面
  const prev = dragGhost.value
  dragGhost.value = null
  const el = document.elementFromPoint(x, y)
  dragGhost.value = prev
  return (el?.closest?.(selector) as HTMLElement | null) || null
}

function findBaseDropZone(x: number, y: number) {
  return findDropZone(x, y, '.mf-ops__draft-base')
}

function findImagesDropZone(x: number, y: number) {
  return findDropZone(x, y, '.mf-ops__draft-images-drop')
}

function endDraftPress() {
  draftPress = null
  draftDragging.value = false
  dragGhost.value = null
  baseDropActive.value = false
  imagesDropActive.value = false
  window.removeEventListener('pointermove', onDraftPointerMove)
  window.removeEventListener('pointerup', onDraftPointerUp)
  window.removeEventListener('pointercancel', onDraftPointerUp)
}

function startImagePress(
  ev: PointerEvent,
  input: {
    url: string
    idx: number
    name: string
    source: DragSource
    pendingId?: string
  },
) {
  if (ev.button !== 0) return
  if ((ev.target as HTMLElement | null)?.closest?.('.mf-ops__draft-image-del')) return
  ev.preventDefault()
  draftPress = {
    pointerId: ev.pointerId,
    startX: ev.clientX,
    startY: ev.clientY,
    url: input.url,
    idx: input.idx,
    name: input.name,
    source: input.source,
    pendingId: input.pendingId,
    dragging: false,
    overBase: false,
    overImages: false,
  }
  window.addEventListener('pointermove', onDraftPointerMove)
  window.addEventListener('pointerup', onDraftPointerUp)
  window.addEventListener('pointercancel', onDraftPointerUp)
}

function onDraftImagePointerDown(ev: PointerEvent, url: string, idx: number) {
  startImagePress(ev, {
    url,
    idx,
    name: idx === 0 ? '封面' : `配图 ${idx + 1}`,
    source: 'draft',
  })
}

function onPendingImagePointerDown(ev: PointerEvent, job: ImageJob, idx: number) {
  if (!job.absPath) return
  startImagePress(ev, {
    url: job.absPath,
    idx,
    name: job.kind === 'edit' ? `编辑 ${idx + 1}` : `生成 ${idx + 1}`,
    source: 'pending',
    pendingId: job.id,
  })
}

function onDraftPointerMove(ev: PointerEvent) {
  if (!draftPress || ev.pointerId !== draftPress.pointerId) return
  const dx = ev.clientX - draftPress.startX
  const dy = ev.clientY - draftPress.startY
  if (!draftPress.dragging) {
    if (dx * dx + dy * dy < 36) return
    draftPress.dragging = true
    draftDragging.value = true
    if (draftPress.source === 'draft') {
      aiTab.value = 'image'
      craftMode.value = 'edit'
    }
    dragGhost.value = {
      x: ev.clientX,
      y: ev.clientY,
      url: draftPress.url,
      idx: draftPress.idx,
      name: draftPress.name,
      source: draftPress.source,
      pendingId: draftPress.pendingId,
    }
  } else if (dragGhost.value) {
    dragGhost.value = {
      ...dragGhost.value,
      x: ev.clientX,
      y: ev.clientY,
    }
  }
  if (!draftPress.dragging) return
  const overBase = Boolean(findBaseDropZone(ev.clientX, ev.clientY))
  const overImages =
    draftPress.source === 'pending' && Boolean(findImagesDropZone(ev.clientX, ev.clientY))
  draftPress.overBase = overBase
  draftPress.overImages = overImages
  baseDropActive.value = overBase
  imagesDropActive.value = overImages
}

async function onDraftPointerUp(ev: PointerEvent) {
  if (!draftPress || ev.pointerId !== draftPress.pointerId) return
  const press = draftPress
  const overBase =
    press.overBase || Boolean(findBaseDropZone(ev.clientX, ev.clientY))
  const overImages =
    press.source === 'pending' &&
    (press.overImages || Boolean(findImagesDropZone(ev.clientX, ev.clientY)))
  const url = press.url
  const idx = press.idx
  endDraftPress()
  if (!press.dragging) return
  ev.preventDefault()
  if (!url) return
  if (press.source === 'pending' && overImages) {
    await addDraftImage(url)
    return
  }
  if (overBase) {
    pickBaseFromDraft(url, Math.max(0, idx))
    ElMessage.success('已设为底图')
  }
}

function onFileDragOver(ev: DragEvent, kind: 'base' | 'images') {
  const types = Array.from(ev.dataTransfer?.types || [])
  if (!types.includes('Files')) return
  ev.preventDefault()
  ev.stopPropagation()
  if (kind === 'base') baseDropActive.value = true
  else imagesDropActive.value = true
  if (ev.dataTransfer) ev.dataTransfer.dropEffect = 'copy'
}

function onFileDragLeave(ev: DragEvent, kind: 'base' | 'images') {
  if (draftDragging.value) return
  const related = ev.relatedTarget as Node | null
  const current = ev.currentTarget as HTMLElement | null
  if (current && related && current.contains(related)) return
  if (kind === 'base') baseDropActive.value = false
  else imagesDropActive.value = false
}

/** 仅承接系统文件拖入（本地图片）；正文配图走 Pointer 拖拽 */
function onBaseDragOver(ev: DragEvent) {
  onFileDragOver(ev, 'base')
}

function onBaseDragLeave(ev: DragEvent) {
  onFileDragLeave(ev, 'base')
}

async function onBaseDrop(ev: DragEvent) {
  ev.preventDefault()
  ev.stopPropagation()
  baseDropActive.value = false
  craftMode.value = 'edit'
  aiTab.value = 'image'
  const file = ev.dataTransfer?.files?.[0]
  if (file?.type.startsWith('image/')) {
    try {
      baseImageUrl.value = await readFileAsDataUrl(file)
      baseImageName.value = file.name || '本地底图'
      ElMessage.success('已设为底图')
    } catch (e) {
      ElMessage.error(e instanceof Error ? e.message : String(e))
    }
  }
}

function onImagesDragOver(ev: DragEvent) {
  onFileDragOver(ev, 'images')
}

function onImagesDragLeave(ev: DragEvent) {
  onFileDragLeave(ev, 'images')
}

async function onImagesDrop(ev: DragEvent) {
  ev.preventDefault()
  ev.stopPropagation()
  imagesDropActive.value = false
  if (!selectedDraft.value) {
    ElMessage.warning('请先选择草稿')
    return
  }
  const file = ev.dataTransfer?.files?.[0]
  if (!file?.type.startsWith('image/')) {
    ElMessage.warning('请拖入图片文件')
    return
  }
  try {
    const dataUrl = await readFileAsDataUrl(file)
    const asset = await assetIngest({
      sourceUrl: dataUrl,
      kind: 'image',
      source: 'upload',
      draftId: selectedDraft.value.id,
      platform: props.platform,
      fileName: file.name,
    })
    await addDraftImage(asset.absPath)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

/** 给 Node sidecar 用：把 Tauri asset URL 还原成绝对路径，避免 Node fetch failed */
function toAiImageSource(src: string) {
  const s = String(src || '').trim()
  if (!s) return s
  if (s.startsWith('data:') || s.startsWith('file:') || s.startsWith('/')) return s
  if (/^[A-Za-z]:[\\/]/.test(s)) return s
  try {
    const u = new URL(s)
    if (u.hostname === 'asset.localhost' || u.protocol === 'asset:') {
      return decodeURIComponent(u.pathname || '')
    }
  } catch {
    /* keep original */
  }
  return s
}

async function onAiOptimize(kind: 'copy' | 'image-generate' | 'image-edit') {
  const draft =
    kind === 'copy' ? aiInstruction.value.trim() : activeImagePrompt.value.trim()
  if (!draft) {
    ElMessage.warning(
      kind === 'copy'
        ? '请先填写改写指令草稿'
        : kind === 'image-edit'
          ? '请先填写编辑指令草稿'
          : '请先填写提示词草稿',
    )
    return
  }
  if (kind === 'image-edit' && !baseImageUrl.value) {
    ElMessage.warning('图生图优化需要底图，请先从正文区拖入')
    return
  }
  if (isBusy.value) return
  busyAction.value = kind === 'copy' ? 'optimize-copy' : 'optimize-image'
  try {
    const res = await aiOptimizePrompt({
      kind,
      prompt: draft,
      styleHint:
        kind === 'image-edit'
          ? 'image edit instruction'
          : kind === 'image-generate'
            ? '小红书笔记配图'
            : '小红书文案改写',
      title: title.value,
      body: body.value,
      imageUrl: kind === 'image-edit' ? toAiImageSource(baseImageUrl.value) : undefined,
    })
    if (kind === 'copy') aiInstruction.value = res.prompt
    else activeImagePrompt.value = res.prompt
    if (kind === 'image-edit' && res.usedVision) {
      ElMessage.success('已结合底图理解并优化')
    } else if (kind === 'image-edit') {
      ElMessage.warning('看图优化未成功，已用文本模型润色指令')
    } else {
      ElMessage.success('已应用 AI 优化结果')
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : 'AI 优化失败')
  } finally {
    busyAction.value = null
  }
}

async function onGenerateImage() {
  if (!selectedDraft.value) return
  if (isBusy.value) return
  const prompt = activeImagePrompt.value.trim()
  if (!prompt) {
    ElMessage.warning(isEditMode.value ? '请填写编辑指令' : '请填写配图提示词')
    return
  }
  if (isEditMode.value && !baseImageUrl.value) {
    ElMessage.warning('请先从正文区拖入底图，或点击上传')
    return
  }
  busyAction.value = 'image'
  try {
    const job = await imageJobStart({
      kind: isEditMode.value ? 'edit' : 'generate',
      prompt,
      size: imageSize.value,
      quality: imageQuality.value,
      baseImagePath: isEditMode.value ? toAiImageSource(baseImageUrl.value) : undefined,
      draftId: selectedDraft.value.id,
      platform: props.platform,
    })
    imageJobsStore.upsertLocal(job)
    ElMessage.success('已提交后台任务，完成后可拖入左侧配图区')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busyAction.value = null
  }
}

async function goPublishCenter() {
  if (!selectedDraft.value) {
    ElMessage.warning('请先选择草稿')
    return
  }
  if (!publishReady.value) {
    ElMessage.warning(publishHint.value)
    return
  }
  if (isBusy.value) return
  busyAction.value = 'publish'
  try {
    const row = await draftUpsert({
      id: selectedDraft.value.id,
      platform: props.platform,
      title: title.value,
      body: body.value,
      tagsJson: JSON.stringify(tags.value),
      imageUrlsJson: JSON.stringify(images.value),
      status: 'ready',
    })
    selectedDraftId.value = row.id
    await refresh({ silent: true })
    const path = props.platform === 'xhs' ? '/xhs/publish' : '/douyin/publish'
    await router.push({ path, query: { draftId: row.id } })
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busyAction.value = null
  }
}

watch(selectedDraftId, async () => {
  loadForm(selectedDraft.value)
  titleCandidates.value = []
  await hydrateDraftImageJobs()
})

onMounted(async () => {
  await refresh()
  await hydrateDraftImageJobs()
  await refreshProducts()
})

onBeforeUnmount(() => {
  endDraftPress()
})
</script>

<template>
  <div
    class="mf-ops__draft-page"
    :class="{ 'is-draft-dragging': draftDragging, 'is-draft-busy': isBusy }"
  >
    <div
      v-if="dragGhost"
      class="mf-ops__draft-drag-ghost"
      :style="{ left: `${dragGhost.x}px`, top: `${dragGhost.y}px` }"
    >
      <img :src="mediaDisplaySrc(dragGhost.url)" alt="" referrerpolicy="no-referrer" />
      <span>{{ dragGhost.name }}</span>
    </div>
    <div class="mf-ops__draft-modes">
      <button
        type="button"
        class="mf-ops__draft-mode"
        :class="{ 'is-active': draftMode === 'rewrite' }"
        @click="draftMode = 'rewrite'"
      >
        改写已有草稿
      </button>
      <button
        type="button"
        class="mf-ops__draft-mode"
        :class="{ 'is-active': draftMode === 'generate' }"
        @click="draftMode = 'generate'"
      >
        生成新草稿
      </button>
    </div>

    <div class="mf-ops__draft-grid">
      <!-- 左：队列 / 生成 -->
      <section class="mf-ops__panel mf-ops__draft-col">
        <div class="mf-ops__panel-head">
          <strong>{{ draftMode === 'generate' ? 'AI 生成' : '草稿队列' }}</strong>
          <el-button
            v-if="draftMode === 'rewrite'"
            size="small"
            type="primary"
            @click="onNewDraft"
          >
            新建
          </el-button>
        </div>
        <div class="mf-ops__panel-body mf-ops__draft-queue" v-loading="loading">
          <template v-if="draftMode === 'generate'">
            <form class="mf-ops__draft-generate" @submit.prevent="onGenerateDraft">
              <div class="mf-ops__draft-field">
                <label>主题</label>
                <el-input v-model="generateTopic" placeholder="如：秋冬干皮护肤清单" />
              </div>
              <div class="mf-ops__draft-field">
                <label>参考（可选）</label>
                <el-input
                  v-model="generateReference"
                  type="textarea"
                  :rows="3"
                  placeholder="粘贴参考笔记或要点"
                />
              </div>
              <el-button
                type="primary"
                native-type="submit"
                :loading="busyAction === 'generate'"
                :disabled="isBusy && busyAction !== 'generate'"
                style="width: 100%"
              >
                AI 生成草稿
              </el-button>

              <div class="mf-ops__product-lib">
                <div class="mf-ops__product-lib-head">
                  <strong>我的产品库</strong>
                  <div class="mf-ops__product-lib-actions">
                    <el-select
                      v-if="productCategories.length"
                      v-model="productCategoryFilter"
                      clearable
                      size="small"
                      placeholder="分类"
                      style="width: 110px"
                    >
                      <el-option
                        v-for="c in productCategories"
                        :key="c"
                        :label="c"
                        :value="c"
                      />
                    </el-select>
                    <el-button
                      size="small"
                      @click="
                        resetProductUploadForm();
                        productUploadVisible = true
                      "
                    >
                      上传
                    </el-button>
                  </div>
                </div>
                <div class="mf-ops__product-grid" v-loading="productsLoading">
                  <div
                    v-for="p in pagedProducts"
                    :key="p.sku"
                    class="mf-ops__product-card"
                    :class="{ 'is-active': selectedProductSku === p.sku }"
                    role="button"
                    tabindex="0"
                    @click="selectProduct(p)"
                    @keydown.enter.prevent="selectProduct(p)"
                  >
                    <img
                      :src="mediaDisplaySrc(p.imageAbsPath)"
                      :alt="p.name"
                      referrerpolicy="no-referrer"
                    />
                    <span class="mf-ops__product-card-name">{{ p.name }}</span>
                    <em v-if="p.category" class="mf-ops__product-card-cat">{{
                      p.category
                    }}</em>
                    <button
                      v-if="p.source === 'user'"
                      type="button"
                      class="mf-ops__product-card-del"
                      title="删除"
                      @click="onDeleteProduct(p, $event)"
                    >
                      ×
                    </button>
                  </div>
                  <div v-if="!productsLoading && !filteredProducts.length" class="mf-ops__product-empty">
                    暂无产品，可点击上传添加
                  </div>
                </div>
                <div v-if="productPagerVisible" class="mf-ops__product-pager">
                  <el-pagination
                    small
                    background
                    layout="prev, pager, next"
                    :total="filteredProducts.length"
                    :page-size="PRODUCT_PAGE_SIZE"
                    :current-page="productPage"
                    @current-change="(p: number) => (productPage = p)"
                  />
                </div>
              </div>
            </form>

            <el-dialog
              v-model="productUploadVisible"
              title="上传产品"
              width="420px"
              append-to-body
              destroy-on-close
            >
              <div class="mf-ops__product-upload">
                <label class="mf-ops__product-upload-img">
                  <img
                    v-if="productUploadForm.previewUrl"
                    :src="productUploadForm.previewUrl"
                    alt=""
                  />
                  <span v-else>点击选择产品图</span>
                  <input type="file" accept="image/*" hidden @change="onProductFileChange" />
                </label>
                <div class="mf-ops__draft-field">
                  <label>品名</label>
                  <el-input v-model="productUploadForm.name" placeholder="必填" />
                </div>
                <div class="mf-ops__draft-field">
                  <label>货号</label>
                  <el-input v-model="productUploadForm.sku" placeholder="可选，留空自动生成" />
                </div>
                <div class="mf-ops__draft-field">
                  <label>规格</label>
                  <el-input v-model="productUploadForm.spec" placeholder="可选" />
                </div>
                <div class="mf-ops__draft-field">
                  <label>分类</label>
                  <el-input v-model="productUploadForm.category" placeholder="可选" />
                </div>
                <div class="mf-ops__draft-field">
                  <label>单位</label>
                  <el-input v-model="productUploadForm.unit" placeholder="如：对 / 条" />
                </div>
              </div>
              <template #footer>
                <el-button @click="productUploadVisible = false">取消</el-button>
                <el-button type="primary" :loading="productUploading" @click="submitProductUpload">
                  保存
                </el-button>
              </template>
            </el-dialog>
          </template>
          <template v-else>
            <button
              v-for="d in drafts"
              :key="d.id"
              type="button"
              class="mf-ops__draft-item"
              :class="{ 'is-active': d.id === selectedDraftId }"
              @click="selectedDraftId = d.id"
            >
              <div class="mf-ops__draft-item-main">
                <strong>{{ d.title || '未命名' }}</strong>
                <em>
                  {{ formatTime(d.updatedAt) }}
                  <template v-if="d.status === 'ready'"> · 待发布</template>
                  <template v-else-if="d.status === 'filled'"> · 已填表</template>
                  <template v-else-if="d.status === 'published'"> · 已发布</template>
                </em>
              </div>
              <el-button size="small" text type="danger" @click.stop="onDeleteDraft(d.id)">
                删
              </el-button>
            </button>
            <div v-if="!drafts.length" class="mf-ops__empty mf-ops__empty--compact">
              <strong>暂无草稿</strong>
              <p>从内容库转入或新建一条。</p>
            </div>
          </template>
        </div>
      </section>

      <!-- 中：编辑器 -->
      <section class="mf-ops__panel mf-ops__draft-col mf-ops__draft-col--editor">
        <div class="mf-ops__panel-head">
          <strong>正文编辑</strong>
          <div class="mf-ops__panel-head-actions">
            <el-button size="small" :disabled="!selectedDraft || isBusy" @click="onSaveDraft()">
              保存
            </el-button>
            <el-button
              size="small"
              type="primary"
              :disabled="!selectedDraft || !publishReady || (isBusy && busyAction !== 'publish')"
              :loading="busyAction === 'publish'"
              @click="goPublishCenter"
            >
              进入发布中心
            </el-button>
          </div>
        </div>
        <div class="mf-ops__panel-body">
          <div v-if="selectedDraft" class="mf-ops__draft-editor">
            <div class="mf-ops__draft-field">
              <div class="mf-ops__draft-field-head">
                <label>标题</label>
                <span class="mf-ops__draft-title-count">{{ title.length }}/20</span>
              </div>
              <el-input v-model="title" maxlength="40" show-word-limit />
            </div>
            <div class="mf-ops__draft-field">
              <label>正文</label>
              <el-input v-model="body" type="textarea" :rows="12" />
            </div>
            <div class="mf-ops__draft-field">
              <label>标签</label>
              <div class="mf-ops__draft-tags">
                <el-tag
                  v-for="tag in tags"
                  :key="tag"
                  closable
                  size="small"
                  @close="removeDraftTag(tag)"
                >
                  #{{ tag }}
                </el-tag>
                <span class="mf-ops__draft-tag-add">
                  <el-input
                    v-model="newTagInput"
                    size="small"
                    placeholder="新标签"
                    @keyup.enter="addDraftTag"
                  />
                  <el-button size="small" @click="addDraftTag">添加</el-button>
                </span>
              </div>
            </div>
            <div class="mf-ops__draft-field">
              <label>配图</label>
              <p class="mf-ops__draft-image-hint">
                右侧生成结果拖入此处加入草稿；已有配图可拖到右侧「底图」做图生图。也可拖入本地图片。第一张为封面。
              </p>
              <div
                class="mf-ops__draft-images-drop"
                :class="{ 'is-drop': imagesDropActive }"
                @dragenter.prevent="onImagesDragOver"
                @dragover.prevent="onImagesDragOver"
                @dragleave="onImagesDragLeave"
                @drop.prevent="onImagesDrop"
              >
                <div v-if="images.length" class="mf-ops__draft-images">
                  <div
                    v-for="(url, idx) in images"
                    :key="`${idx}-${url.slice(0, 48)}`"
                    class="mf-ops__draft-image"
                    :class="{
                      'is-dragging':
                        draftDragging &&
                        dragGhost?.source === 'draft' &&
                        dragGhost?.idx === idx,
                    }"
                    title="按住拖到右侧「底图」用作图生图"
                    @pointerdown="onDraftImagePointerDown($event, url, idx)"
                  >
                    <img
                      :src="mediaDisplaySrc(url)"
                      :alt="`配图 ${idx + 1}`"
                      loading="lazy"
                      draggable="false"
                      referrerpolicy="no-referrer"
                    />
                    <em>{{ idx === 0 ? '封面' : idx + 1 }}</em>
                    <button
                      type="button"
                      class="mf-ops__draft-image-del"
                      @click.stop="removeDraftImage(url)"
                      @pointerdown.stop
                    >
                      ×
                    </button>
                  </div>
                </div>
                <p v-else class="mf-ops__draft-assist">
                  {{
                    imagesDropActive
                      ? '松开以加入配图'
                      : '暂无配图 · 将右侧生成缩略图拖入此处，或拖入本地图片'
                  }}
                </p>
              </div>
            </div>
          </div>
          <div v-else class="mf-ops__empty mf-ops__empty--compact">
            <strong>选择或新建草稿</strong>
            <p>在左侧队列选择一条草稿开始编辑。</p>
          </div>
        </div>
      </section>

      <!-- 右：AI Dock -->
      <section class="mf-ops__panel mf-ops__draft-col mf-ops__draft-col--ai">
        <div class="mf-ops__panel-body mf-ops__draft-ai">
          <div class="mf-ops__draft-ai-tabs">
            <button
              type="button"
              class="mf-ops__draft-ai-tab"
              :class="{ 'is-on': aiTab === 'copy' }"
              @click="aiTab = 'copy'"
            >
              文案
            </button>
            <button
              type="button"
              class="mf-ops__draft-ai-tab"
              :class="{ 'is-on': aiTab === 'image' }"
              @click="aiTab = 'image'"
            >
              图片
            </button>
          </div>

          <div class="mf-ops__draft-ai-scroll">
            <section v-show="aiTab === 'copy'" class="mf-ops__draft-ai-section">
              <div class="mf-ops__draft-field">
                <div class="mf-ops__draft-field-head">
                  <label>改写指令（可选）</label>
                  <button
                    type="button"
                    class="mf-ops__draft-ai-opt"
                    :disabled="isBusy && busyAction !== 'optimize-copy'"
                    @click="onAiOptimize('copy')"
                  >
                    {{ busyAction === 'optimize-copy' ? '优化中…' : '✨ AI 优化' }}
                  </button>
                </div>
                <el-input
                  v-model="aiInstruction"
                  type="textarea"
                  :rows="3"
                  :disabled="busyAction === 'optimize-copy'"
                  placeholder="如：更口语化、加入 emoji"
                />
              </div>
              <div class="mf-ops__draft-prompt-presets">
                <button
                  type="button"
                  class="mf-ops__draft-prompt-preset"
                  :class="{ 'is-busy': busyAction === 'rewrite' }"
                  :disabled="!selectedDraft || (isBusy && busyAction !== 'rewrite')"
                  @click="runDraftAi('rewrite')"
                >
                  <strong>{{ busyAction === 'rewrite' ? '改写中…' : 'AI 改写正文' }}</strong>
                  <em>保留结构优化表达</em>
                </button>
                <button
                  type="button"
                  class="mf-ops__draft-prompt-preset"
                  :class="{ 'is-busy': busyAction === 'titles' }"
                  :disabled="!selectedDraft || (isBusy && busyAction !== 'titles')"
                  @click="runDraftAi('titles')"
                >
                  <strong>{{ busyAction === 'titles' ? '生成中…' : '生成标题' }}</strong>
                  <em>小红书风格标题</em>
                </button>
                <button
                  type="button"
                  class="mf-ops__draft-prompt-preset"
                  :class="{ 'is-busy': busyAction === 'tags' }"
                  :disabled="!selectedDraft || (isBusy && busyAction !== 'tags')"
                  @click="runDraftAi('tags')"
                >
                  <strong>{{ busyAction === 'tags' ? '生成中…' : '生成标签' }}</strong>
                  <em>热门话题标签</em>
                </button>
                <button
                  type="button"
                  class="mf-ops__draft-prompt-preset"
                  disabled
                  title="需配置多轮对话 API"
                >
                  <strong>对话助手</strong>
                  <em>本期简化</em>
                </button>
              </div>
              <div v-if="titleCandidates.length" class="mf-ops__draft-title-picks">
                <div class="mf-ops__draft-title-picks-head">
                  <strong>候选标题</strong>
                  <button
                    type="button"
                    class="mf-ops__draft-title-picks-clear"
                    @click="titleCandidates = []"
                  >
                    清空
                  </button>
                </div>
                <button
                  v-for="(t, i) in titleCandidates"
                  :key="`${i}-${t}`"
                  type="button"
                  class="mf-ops__draft-title-pick"
                  :disabled="titlePickBusy"
                  @click="applyTitleCandidate(t)"
                >
                  <span>{{ t }}</span>
                  <em>选用</em>
                </button>
              </div>
              <p class="mf-ops__draft-ai-note">需先在「模型配置」中设置文本大模型。</p>
            </section>

            <section v-show="aiTab === 'image'" class="mf-ops__draft-ai-section mf-ops__draft-ai-image">
              <div class="mf-ops__draft-craft-seg">
                <button
                  type="button"
                  class="mf-ops__draft-craft-btn"
                  :class="{ 'is-on': craftMode === 'generate' }"
                  :disabled="busyAction === 'image'"
                  @click="setCraftMode('generate')"
                >
                  文生图
                </button>
                <button
                  type="button"
                  class="mf-ops__draft-craft-btn"
                  :class="{ 'is-on': craftMode === 'edit' }"
                  :disabled="busyAction === 'image'"
                  @click="setCraftMode('edit')"
                >
                  图生图
                </button>
              </div>

              <!-- v-show：拖拽过程中勿用 v-if 重建落点，否则 WKWebView 会丢 drop -->
              <div
                v-show="isEditMode || (draftDragging && dragGhost?.source === 'draft')"
                class="mf-ops__draft-base"
                :class="{ 'is-drop': baseDropActive }"
                @dragenter.prevent="onBaseDragOver"
                @dragover.prevent="onBaseDragOver"
                @dragleave="onBaseDragLeave"
                @drop.prevent="onBaseDrop"
              >
                <div class="mf-ops__draft-base-head">
                  <label>底图</label>
                  <div class="mf-ops__draft-base-actions">
                    <button
                      type="button"
                      class="mf-ops__draft-base-btn"
                      :disabled="busyAction === 'image'"
                      @click="openBaseFilePicker"
                    >
                      上传
                    </button>
                    <button
                      v-if="baseImageUrl"
                      type="button"
                      class="mf-ops__draft-base-btn"
                      :disabled="busyAction === 'image'"
                      @click="clearBaseImage"
                    >
                      清除
                    </button>
                  </div>
                </div>
                <input
                  ref="baseFileInput"
                  type="file"
                  accept="image/*"
                  class="mf-ops__draft-file-hidden"
                  @change="onBaseFileChange"
                />
                <div v-if="baseImageUrl" class="mf-ops__draft-base-card">
                  <img :src="mediaDisplaySrc(baseImageUrl)" alt="底图" referrerpolicy="no-referrer" />
                  <div>
                    <strong>{{ baseImageName || '底图' }}</strong>
                    <em>可从正文区拖入配图替换，或上传本地文件</em>
                  </div>
                </div>
                <div v-else class="mf-ops__draft-base-empty">
                  {{
                    baseDropActive
                      ? '松开以设为底图'
                      : '从正文区按住拖入配图，或点击上方「上传」'
                  }}
                </div>
              </div>

              <div class="mf-ops__draft-field">
                <div class="mf-ops__draft-field-head">
                  <label>{{ isEditMode ? '编辑指令' : '正向提示词' }}</label>
                  <button
                    type="button"
                    class="mf-ops__draft-ai-opt"
                    :disabled="
                      (isBusy && busyAction !== 'optimize-image') || (isEditMode && !baseImageUrl)
                    "
                    @click="onAiOptimize(isEditMode ? 'image-edit' : 'image-generate')"
                  >
                    {{ busyAction === 'optimize-image' ? '优化中…' : '✨ AI 优化' }}
                  </button>
                </div>
                <el-input
                  v-model="activeImagePrompt"
                  type="textarea"
                  :rows="4"
                  :disabled="busyAction === 'optimize-image' || busyAction === 'image'"
                  :placeholder="
                    isEditMode
                      ? '描述要如何修改这张图，例如：手模佩戴特写、耳部试戴…'
                      : '描述想要的笔记配图风格与主体'
                  "
                />
                <div v-if="isEditMode" class="mf-ops__draft-chips mf-ops__draft-wear-presets">
                  <button
                    v-for="p in JEWELRY_WEAR_PRESETS"
                    :key="p.label"
                    type="button"
                    class="mf-ops__draft-chip"
                    :class="{ 'is-on': editPrompt === p.prompt }"
                    :disabled="busyAction === 'optimize-image' || busyAction === 'image'"
                    :title="p.prompt"
                    @click="editPrompt = p.prompt"
                  >
                    {{ p.label }}
                  </button>
                </div>
              </div>

              <div class="mf-ops__draft-field">
                <label>比例 / 尺寸</label>
                <div class="mf-ops__draft-chips">
                  <button
                    v-for="r in IMAGE_RATIOS"
                    :key="r.size"
                    type="button"
                    class="mf-ops__draft-chip"
                    :class="{ 'is-on': imageSize === r.size }"
                    @click="imageSize = r.size"
                  >
                    {{ r.label }}
                  </button>
                </div>
              </div>

              <div class="mf-ops__draft-field">
                <label>质量</label>
                <div class="mf-ops__draft-chips">
                  <button
                    v-for="q in IMAGE_QUALITIES"
                    :key="q.value"
                    type="button"
                    class="mf-ops__draft-chip"
                    :class="{ 'is-on': imageQuality === q.value }"
                    @click="imageQuality = q.value"
                  >
                    {{ q.label }}
                  </button>
                </div>
              </div>

              <el-button
                type="primary"
                :loading="busyAction === 'image'"
                :disabled="
                  !selectedDraft ||
                  (isEditMode && !baseImageUrl) ||
                  (isBusy && busyAction !== 'image')
                "
                style="width: 100%"
                @click="onGenerateImage"
              >
                生成配图
              </el-button>

              <div
                v-if="activeImageJobs.length || pendingDoneJobs.length || draftImageJobs.some((j) => j.status === 'error')"
                class="mf-ops__draft-pending"
              >
                <div class="mf-ops__draft-pending-head">
                  <label>生成结果</label>
                  <span>后台任务完成后可拖入左侧配图区</span>
                </div>
                <div class="mf-ops__draft-pending-grid">
                  <div
                    v-for="job in activeImageJobs"
                    :key="job.id"
                    class="mf-ops__draft-image mf-ops__draft-pending-item is-job"
                  >
                    <div class="mf-ops__draft-job-status">
                      <strong>{{ jobStatusLabel(job.status) }}</strong>
                      <em>{{ job.kind === 'edit' ? '图生图' : '文生图' }}</em>
                    </div>
                  </div>
                  <div
                    v-for="(job, idx) in pendingDoneJobs"
                    :key="job.id"
                    class="mf-ops__draft-image mf-ops__draft-pending-item"
                    :class="{
                      'is-dragging':
                        draftDragging &&
                        dragGhost?.source === 'pending' &&
                        dragGhost?.pendingId === job.id,
                    }"
                    title="按住拖到左侧配图区"
                    @pointerdown="onPendingImagePointerDown($event, job, idx)"
                  >
                    <img
                      :src="mediaDisplaySrc(job.absPath)"
                      :alt="job.prompt"
                      loading="lazy"
                      draggable="false"
                      referrerpolicy="no-referrer"
                    />
                    <em>{{ job.kind === 'edit' ? '编辑' : '生成' }} {{ idx + 1 }}</em>
                    <button
                      type="button"
                      class="mf-ops__draft-pending-use"
                      @click.stop="useJobImage(job)"
                      @pointerdown.stop
                    >
                      加入
                    </button>
                  </div>
                  <div
                    v-for="job in draftImageJobs.filter((j) => j.status === 'error')"
                    :key="job.id"
                    class="mf-ops__draft-image mf-ops__draft-pending-item is-job is-error"
                    :title="job.error || '失败'"
                  >
                    <div class="mf-ops__draft-job-status">
                      <strong>失败</strong>
                      <em>{{ job.error || '生成失败' }}</em>
                    </div>
                  </div>
                </div>
              </div>

              <p class="mf-ops__draft-ai-note">
                提交后后台执行，可离开本页；完成后会出现在上方与素材中心，拖入左侧配图区后才写入草稿。
              </p>
            </section>

            <div class="mf-ops__draft-next">
              <p class="mf-ops__draft-assist">{{ publishHint }}</p>
              <el-button
                type="primary"
                class="mf-ops__draft-send"
                :disabled="!selectedDraft || !publishReady || (isBusy && busyAction !== 'publish')"
                :loading="busyAction === 'publish'"
                @click="goPublishCenter"
              >
                下一步：进入发布中心
              </el-button>
            </div>
          </div>
        </div>
      </section>
    </div>
  </div>
</template>
