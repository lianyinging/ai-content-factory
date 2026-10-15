<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import type { UnlistenFn } from '@tauri-apps/api/event'
import {
  kbCapability,
  kbCreateBase,
  kbDeleteBase,
  kbDeleteQa,
  kbImportQa,
  kbListBases,
  kbListQa,
  kbSearch,
  kbUpsertQa,
  type KbBase,
  type KbQa,
  type KbSearchHit,
} from '@/api/knowledge'
import { readTextFile, saveTextWithDialog } from '@/api/fsDialog'

const loading = ref(false)
const capMsg = ref('')
const capOk = ref(false)
const bases = ref<KbBase[]>([])
const selectedBaseId = ref('')
const qaItems = ref<KbQa[]>([])
const qaTotal = ref(0)
const qaPage = ref(1)
const qaPageSize = ref(20)
const qaFilter = ref('')
const qaLoading = ref(false)

const createBaseOpen = ref(false)
const baseForm = reactive({ name: '', description: '' })
const creatingBase = ref(false)

const qaDialogOpen = ref(false)
const editingQaId = ref('')
const qaForm = reactive({ question: '', answer: '', tags: '' })
const savingQa = ref(false)

const importOpen = ref(false)
const importText = ref('')
const importFileName = ref('')
const importDragging = ref(false)
const importing = ref(false)
const importFileInput = ref<HTMLInputElement | null>(null)
const importDropEl = ref<HTMLElement | null>(null)
/** HTML5 dragenter/leave 嵌套计数，避免子元素触发误清除 */
const importDragDepth = ref(0)
let unlistenImportDrop: UnlistenFn | null = null

const trialQuery = ref('')
const trialBusy = ref(false)
const trialHits = ref<KbSearchHit[]>([])

const selectedBase = computed(() => bases.value.find((b) => b.id === selectedBaseId.value) || null)

const TXT_TEMPLATE = `Q: 不激活要收费吗？
A: 不要

Q: 话费补贴什么时候可以领？
A: 从激活后的次月开始
`

const EXCEL_TEMPLATE =
  '\uFEFFquestion,answer,tags\n' +
  '不激活要收费吗？,不要,\n' +
  '话费补贴什么时候可以领？,从激活后的次月开始,\n'

async function downloadTemplate(opts: {
  defaultName: string
  content: string
  filterName: string
  ext: string
}) {
  try {
    const path = await saveTextWithDialog({
      defaultName: opts.defaultName,
      content: opts.content,
      filterName: opts.filterName,
      extensions: [opts.ext],
    })
    if (!path) return
    ElMessage.success(`模板已保存：${path}`)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function downloadTxtTemplate() {
  await downloadTemplate({
    defaultName: '知识库导入模板.txt',
    content: TXT_TEMPLATE,
    filterName: '文本文件',
    ext: 'txt',
  })
}

async function downloadExcelTemplate() {
  await downloadTemplate({
    defaultName: '知识库导入模板.csv',
    content: EXCEL_TEMPLATE,
    filterName: 'CSV / Excel',
    ext: 'csv',
  })
}

function resetImportDialog() {
  importText.value = ''
  importFileName.value = ''
  importDragging.value = false
  importDragDepth.value = 0
}

function openImportDialog() {
  resetImportDialog()
  importOpen.value = true
}

function basenameOf(pathOrName: string) {
  const s = String(pathOrName || '').trim()
  const parts = s.split(/[/\\]/)
  return parts[parts.length - 1] || s
}

function validateImportName(name: string, mime = ''): string | null {
  const lower = name.toLowerCase()
  if (lower.endsWith('.xlsx') || lower.endsWith('.xls')) {
    return '请使用「下载 Excel 模板」得到的 CSV，或另存为 CSV / TXT 后再上传'
  }
  if (
    !/\.(txt|csv|json|md)$/i.test(lower) &&
    mime &&
    !mime.startsWith('text/') &&
    mime !== 'application/json'
  ) {
    return '请拖入 .txt / .csv / .json / .md 文件'
  }
  if (!/\.(txt|csv|json|md)$/i.test(lower) && !mime) {
    return '请拖入 .txt / .csv / .json / .md 文件'
  }
  return null
}

async function applyImportContent(name: string, content: string) {
  importText.value = content
  importFileName.value = name
  ElMessage.success(`已载入 ${name}`)
}

async function readImportFile(file: File) {
  const err = validateImportName(file.name, file.type)
  if (err) {
    ElMessage.warning(err)
    return
  }
  try {
    await applyImportContent(file.name, await file.text())
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : '读取文件失败')
  }
}

async function readImportPath(path: string) {
  const name = basenameOf(path)
  const err = validateImportName(name)
  if (err) {
    ElMessage.warning(err)
    return
  }
  try {
    const content = await readTextFile(path)
    await applyImportContent(name, content)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : '读取文件失败')
  }
}

function onImportFilePick(ev: Event) {
  const input = ev.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = ''
  if (file) void readImportFile(file)
}

function onImportDragEnter(ev: DragEvent) {
  ev.preventDefault()
  ev.stopPropagation()
  importDragDepth.value += 1
  importDragging.value = true
}

function onImportDragOver(ev: DragEvent) {
  ev.preventDefault()
  ev.stopPropagation()
  if (ev.dataTransfer) ev.dataTransfer.dropEffect = 'copy'
  importDragging.value = true
}

function onImportDragLeave(ev: DragEvent) {
  ev.preventDefault()
  ev.stopPropagation()
  importDragDepth.value = Math.max(0, importDragDepth.value - 1)
  if (importDragDepth.value === 0) importDragging.value = false
}

function onImportDrop(ev: DragEvent) {
  ev.preventDefault()
  ev.stopPropagation()
  importDragDepth.value = 0
  importDragging.value = false
  const file = ev.dataTransfer?.files?.[0]
  if (file) void readImportFile(file)
}

/** 判断 Tauri 物理坐标是否落在导入虚线框内 */
function isPointOverImportDrop(physical: { x: number; y: number }) {
  const el = importDropEl.value
  if (!el) return true
  const rect = el.getBoundingClientRect()
  const dpr = window.devicePixelRatio || 1
  const x = physical.x / dpr
  const y = physical.y / dpr
  return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
}

async function bindImportTauriDrop() {
  unlistenImportDrop?.()
  unlistenImportDrop = null
  try {
    unlistenImportDrop = await getCurrentWebview().onDragDropEvent((event) => {
      if (!importOpen.value) return
      const payload = event.payload
      if (payload.type === 'enter' || payload.type === 'over') {
        importDragging.value = isPointOverImportDrop(payload.position)
        return
      }
      if (payload.type === 'leave') {
        importDragging.value = false
        return
      }
      if (payload.type === 'drop') {
        importDragging.value = false
        // 导入弹窗打开时，落到窗口内任意位置都接受（虚线框外也算）
        const path = payload.paths?.[0]
        if (path) void readImportPath(path)
      }
    })
  } catch {
    /* 非 Tauri 环境忽略 */
  }
}

function unbindImportTauriDrop() {
  unlistenImportDrop?.()
  unlistenImportDrop = null
}

async function refreshCapability() {
  try {
    const cap = await kbCapability()
    capOk.value = Boolean(cap.available)
    const parts = [
      cap.message,
      cap.embedding?.message,
      cap.chroma?.message,
      !cap.embedding?.available ? `模型路径：${cap.embedding?.path || ''}` : '',
    ].filter(Boolean)
    capMsg.value = parts.join(' · ') || (capOk.value ? 'QA 知识库就绪' : '知识库能力未就绪')
  } catch (e) {
    capOk.value = false
    capMsg.value = e instanceof Error ? e.message : '无法读取能力'
  }
}

async function loadBases() {
  loading.value = true
  try {
    await refreshCapability()
    bases.value = await kbListBases()
    if (!selectedBaseId.value && bases.value[0]) selectedBaseId.value = bases.value[0].id
    if (selectedBaseId.value && !bases.value.some((b) => b.id === selectedBaseId.value)) {
      selectedBaseId.value = bases.value[0]?.id || ''
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : '加载失败')
  } finally {
    loading.value = false
  }
}

async function loadQa() {
  if (!selectedBaseId.value) {
    qaItems.value = []
    qaTotal.value = 0
    return
  }
  qaLoading.value = true
  try {
    const data = await kbListQa(
      selectedBaseId.value,
      qaPage.value,
      qaPageSize.value,
      qaFilter.value,
    )
    qaItems.value = data.items
    qaTotal.value = data.total
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : '加载问答失败')
  } finally {
    qaLoading.value = false
  }
}

function searchQa() {
  qaPage.value = 1
  void loadQa()
}

watch(selectedBaseId, () => {
  qaPage.value = 1
  trialHits.value = []
  void loadQa()
})

watch(importOpen, (open) => {
  if (open) void bindImportTauriDrop()
  else unbindImportTauriDrop()
})

onMounted(() => {
  void loadBases().then(() => loadQa())
})

onUnmounted(() => {
  unbindImportTauriDrop()
})

async function onCreateBase() {
  if (!baseForm.name.trim()) {
    ElMessage.warning('请填写知识库名称')
    return
  }
  creatingBase.value = true
  try {
    const item = await kbCreateBase(baseForm.name, baseForm.description)
    createBaseOpen.value = false
    baseForm.name = ''
    baseForm.description = ''
    await loadBases()
    selectedBaseId.value = item.id
    ElMessage.success('已创建知识库')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : '创建失败')
  } finally {
    creatingBase.value = false
  }
}

async function onDeleteBase(base: KbBase) {
  try {
    await ElMessageBox.confirm(`确定删除知识库「${base.name}」及全部问答？`, '确认', {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    await kbDeleteBase(base.id)
    if (selectedBaseId.value === base.id) selectedBaseId.value = ''
    await loadBases()
    await loadQa()
    ElMessage.success('已删除')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : '删除失败')
  }
}

function openCreateQa() {
  editingQaId.value = ''
  qaForm.question = ''
  qaForm.answer = ''
  qaForm.tags = ''
  qaDialogOpen.value = true
}

function openEditQa(row: KbQa) {
  editingQaId.value = row.id
  qaForm.question = row.question
  qaForm.answer = row.answer
  qaForm.tags = (row.tags || []).join(', ')
  qaDialogOpen.value = true
}

async function onSaveQa() {
  if (!selectedBaseId.value) return
  if (!qaForm.question.trim() || !qaForm.answer.trim()) {
    ElMessage.warning('问题和答案不能为空')
    return
  }
  const tags = qaForm.tags
    .split(/[,，]/)
    .map((s) => s.trim())
    .filter(Boolean)
  savingQa.value = true
  try {
    await kbUpsertQa({
      baseId: selectedBaseId.value,
      qaId: editingQaId.value || undefined,
      question: qaForm.question,
      answer: qaForm.answer,
      tags,
    })
    qaDialogOpen.value = false
    await loadBases()
    await loadQa()
    ElMessage.success('已保存')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : '保存失败')
  } finally {
    savingQa.value = false
  }
}

async function onDeleteQa(row: KbQa) {
  if (!selectedBaseId.value) return
  try {
    await ElMessageBox.confirm('确定删除这条问答？', '确认', { type: 'warning' })
  } catch {
    return
  }
  try {
    await kbDeleteQa(selectedBaseId.value, row.id)
    await loadBases()
    await loadQa()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : '删除失败')
  }
}

async function onImport() {
  if (!selectedBaseId.value || !importText.value.trim()) {
    ElMessage.warning('请先选择文件或粘贴导入内容')
    return
  }
  importing.value = true
  try {
    const res = await kbImportQa(selectedBaseId.value, importText.value)
    importOpen.value = false
    resetImportDialog()
    await loadBases()
    await loadQa()
    ElMessage.success(`已导入 ${Number(res.created || 0)} 条`)
    const errors = Array.isArray(res.errors) ? (res.errors as string[]) : []
    if (errors.length) {
      ElMessage.warning(errors.slice(0, 3).join('；'))
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : '导入失败')
  } finally {
    importing.value = false
  }
}

async function onTrialSearch() {
  if (!selectedBaseId.value) {
    ElMessage.warning('请先选择知识库')
    return
  }
  if (!trialQuery.value.trim()) {
    ElMessage.warning('请输入试问内容')
    return
  }
  if (!capOk.value) {
    ElMessage.warning(capMsg.value || '知识库能力未就绪')
    return
  }
  trialBusy.value = true
  try {
    trialHits.value = await kbSearch([selectedBaseId.value], trialQuery.value, 5)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : '检索失败')
  } finally {
    trialBusy.value = false
  }
}
</script>

<template>
  <div class="knowledge-page" v-loading="loading">
    <header class="kb-head">
      <h1>知识库</h1>
      <p>维护本机 QA 知识库，供自动运营检索增强回复。向量检索依赖本地 BGE + Chroma。</p>
      <p class="kb-cap" :class="{ ok: capOk, bad: !capOk }">{{ capMsg }}</p>
    </header>

    <div class="kb-body">
      <aside class="kb-sider">
        <div class="kb-sider-top">
          <span>知识库</span>
          <el-button type="primary" size="small" @click="createBaseOpen = true">新建</el-button>
        </div>
        <ul class="kb-base-list">
          <li
            v-for="b in bases"
            :key="b.id"
            class="kb-base-item"
            :class="{ on: b.id === selectedBaseId }"
            @click="selectedBaseId = b.id"
          >
            <div class="kb-base-name">{{ b.name }}</div>
            <div class="kb-base-meta">{{ b.qaCount || 0 }} 条问答</div>
            <el-button class="kb-base-del" link type="danger" @click.stop="onDeleteBase(b)">
              删除
            </el-button>
          </li>
          <li v-if="!bases.length" class="kb-empty">暂无知识库，请先新建</li>
        </ul>
      </aside>

      <section class="kb-main">
        <div v-if="!selectedBase" class="kb-empty-main">请先选择左侧知识库</div>
        <template v-else>
          <div class="kb-toolbar">
            <div class="kb-toolbar-title">{{ selectedBase.name }}</div>
            <div class="kb-toolbar-actions">
              <el-input
                v-model="qaFilter"
                clearable
                placeholder="筛选问题/答案"
                class="kb-filter"
                @keyup.enter="searchQa"
                @clear="searchQa"
              />
              <el-button @click="searchQa">搜索</el-button>
              <el-button @click="openImportDialog">导入</el-button>
              <el-button type="primary" @click="openCreateQa">新增问答</el-button>
            </div>
          </div>

          <div class="kb-table-wrap" v-loading="qaLoading">
            <el-table :data="qaItems" empty-text="暂无问答" style="width: 100%" height="100%">
              <el-table-column prop="question" label="问题" min-width="180" show-overflow-tooltip />
              <el-table-column prop="answer" label="答案" min-width="240" show-overflow-tooltip />
              <el-table-column label="标签" width="120">
                <template #default="{ row }">{{ (row.tags || []).join(', ') }}</template>
              </el-table-column>
              <el-table-column label="操作" width="140" fixed="right">
                <template #default="{ row }">
                  <el-button link type="primary" @click="openEditQa(row)">编辑</el-button>
                  <el-button link type="danger" @click="onDeleteQa(row)">删除</el-button>
                </template>
              </el-table-column>
            </el-table>
          </div>

          <div class="kb-pagination">
            <el-pagination
              v-model:current-page="qaPage"
              v-model:page-size="qaPageSize"
              :total="qaTotal"
              :page-sizes="[10, 20, 50]"
              layout="total, sizes, prev, pager, next"
              background
              @current-change="loadQa"
              @size-change="searchQa"
            />
          </div>

          <div class="kb-trial">
            <div class="kb-trial-title">试问检索</div>
            <div class="kb-trial-row">
              <el-input
                v-model="trialQuery"
                placeholder="模拟用户会怎么问…"
                @keyup.enter="onTrialSearch"
              />
              <el-button type="primary" :loading="trialBusy" @click="onTrialSearch">检索</el-button>
            </div>
            <ul v-if="trialHits.length" class="kb-hits">
              <li v-for="hit in trialHits" :key="hit.id" class="kb-hit">
                <div class="kb-hit-q">
                  <strong>{{ hit.question }}</strong>
                  <span class="kb-hit-score">{{ hit.score }}</span>
                </div>
                <p>{{ hit.answer }}</p>
              </li>
            </ul>
            <p v-else-if="!trialBusy" class="kb-hit-empty">输入问题后点击检索，查看语义命中结果</p>
          </div>
        </template>
      </section>
    </div>

    <el-dialog v-model="createBaseOpen" title="新建知识库" width="440px" align-center destroy-on-close>
      <el-form label-position="top">
        <el-form-item label="名称" required>
          <el-input v-model="baseForm.name" maxlength="64" />
        </el-form-item>
        <el-form-item label="描述">
          <el-input v-model="baseForm.description" type="textarea" :rows="2" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="createBaseOpen = false">取消</el-button>
        <el-button type="primary" :loading="creatingBase" @click="onCreateBase">确定</el-button>
      </template>
    </el-dialog>

    <el-dialog
      v-model="qaDialogOpen"
      :title="editingQaId ? '编辑问答' : '新增问答'"
      width="560px"
      align-center
      destroy-on-close
    >
      <el-form label-position="top">
        <el-form-item label="问题" required>
          <el-input v-model="qaForm.question" type="textarea" :rows="2" />
        </el-form-item>
        <el-form-item label="答案" required>
          <el-input v-model="qaForm.answer" type="textarea" :rows="4" />
        </el-form-item>
        <el-form-item label="标签">
          <el-input v-model="qaForm.tags" placeholder="逗号分隔，可选" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="qaDialogOpen = false">取消</el-button>
        <el-button type="primary" :loading="savingQa" @click="onSaveQa">保存</el-button>
      </template>
    </el-dialog>

    <el-dialog
      v-model="importOpen"
      title="导入问答"
      width="560px"
      align-center
      destroy-on-close
      @closed="resetImportDialog"
    >
      <div class="kb-import-toolbar">
        <el-button size="small" @click="downloadTxtTemplate">下载 TXT 模板</el-button>
        <el-button size="small" @click="downloadExcelTemplate">下载 Excel 模板</el-button>
      </div>
      <p class="kb-import-hint">
        支持 Q:/A: 文本块、CSV（question,answer,tags）或 JSON 数组。先下载模板再填写导入。
      </p>
      <div
        ref="importDropEl"
        class="kb-import-drop"
        :class="{ 'is-drag': importDragging }"
        @dragenter="onImportDragEnter"
        @dragover="onImportDragOver"
        @dragleave="onImportDragLeave"
        @drop="onImportDrop"
        @click="importFileInput?.click()"
      >
        <input
          ref="importFileInput"
          type="file"
          class="kb-import-file"
          accept=".txt,.csv,.json,.md,text/plain,text/csv,application/json"
          @change="onImportFilePick"
        />
        <strong>点击或拖拽文件到此处</strong>
        <span>支持 .txt / .csv / .json / .md</span>
        <em v-if="importFileName">{{ importFileName }}</em>
      </div>
      <template #footer>
        <el-button @click="importOpen = false">取消</el-button>
        <el-button type="primary" :loading="importing" :disabled="!importText.trim()" @click="onImport">
          导入
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.knowledge-page {
  --kb-ink: var(--text, #1f2329);
  --kb-mute: var(--muted, #8a919f);
  --kb-line: var(--line, #e5e6eb);
  --kb-panel: var(--panel, #fff);
  --kb-primary: var(--el-color-primary, #2563eb);
  display: flex;
  flex-direction: column;
  height: 100%;
  max-height: 100%;
  min-height: 0;
  overflow: hidden;
  box-sizing: border-box;
  background: var(--kb-panel);
  padding: 16px 20px 14px;
}

.kb-head {
  flex: none;
  padding-bottom: 14px;
  border-bottom: 1px solid var(--kb-line);
}

.kb-head h1 {
  margin: 0;
  font-size: 22px;
  font-weight: 700;
  color: var(--kb-ink);
}

.kb-head p {
  margin: 6px 0 0;
  color: var(--kb-mute);
  font-size: 13px;
  max-width: 42em;
  line-height: 1.5;
}

.kb-cap {
  margin-top: 8px !important;
  font-size: 12px !important;
}

.kb-cap.ok {
  color: #16a34a;
}

.kb-cap.bad {
  color: #dc2626;
}

.kb-body {
  flex: 1;
  min-height: 0;
  display: flex;
}

.kb-sider {
  flex: none;
  width: 220px;
  border-right: 1px solid var(--kb-line);
  display: flex;
  flex-direction: column;
  min-height: 0;
  padding: 12px 12px 12px 0;
}

.kb-sider-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 10px;
  font-weight: 600;
  font-size: 13px;
}

.kb-base-list {
  list-style: none;
  margin: 0;
  padding: 0;
  overflow: auto;
  flex: 1;
  min-height: 0;
}

.kb-base-item {
  position: relative;
  padding: 10px 28px 10px 10px;
  cursor: pointer;
  border-bottom: 1px solid var(--kb-line);
}

.kb-base-item.on {
  background: color-mix(in srgb, var(--kb-primary) 8%, transparent);
}

.kb-base-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--kb-ink);
}

.kb-base-meta {
  font-size: 12px;
  color: var(--kb-mute);
  margin-top: 2px;
}

.kb-base-del {
  position: absolute;
  right: 4px;
  top: 8px;
  opacity: 0;
}

.kb-base-item:hover .kb-base-del {
  opacity: 1;
}

.kb-main {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 12px 0 0 16px;
}

.kb-toolbar {
  flex: none;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--kb-line);
}

.kb-toolbar-title {
  font-size: 16px;
  font-weight: 600;
}

.kb-toolbar-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  align-items: center;
}

.kb-filter {
  width: min(220px, 40vw);
}

.kb-table-wrap {
  flex: 1;
  min-height: 160px;
  margin-top: 12px;
  overflow: hidden;
}

.kb-pagination {
  flex: none;
  display: flex;
  justify-content: flex-end;
  padding-top: 12px;
}

.kb-trial {
  flex: none;
  margin-top: 14px;
  padding-top: 14px;
  border-top: 1px solid var(--kb-line);
}

.kb-trial-title {
  font-weight: 600;
  font-size: 13px;
  margin-bottom: 8px;
}

.kb-trial-row {
  display: flex;
  gap: 8px;
}

.kb-hits {
  list-style: none;
  margin: 10px 0 0;
  padding: 0;
  max-height: 180px;
  overflow: auto;
}

.kb-hit {
  padding: 8px 0;
  border-bottom: 1px solid var(--kb-line);
}

.kb-hit-q {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  font-size: 13px;
}

.kb-hit-score {
  color: var(--kb-mute);
  font-variant-numeric: tabular-nums;
}

.kb-hit p {
  margin: 4px 0 0;
  font-size: 13px;
  color: var(--kb-mute);
  line-height: 1.45;
}

.kb-hit-empty,
.kb-empty,
.kb-empty-main {
  color: var(--kb-mute);
  font-size: 13px;
  padding: 16px 0;
}

.kb-import-hint {
  margin: 0 0 10px;
  font-size: 13px;
  color: var(--kb-mute);
  line-height: 1.5;
}

.kb-import-toolbar {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 10px;
}

.kb-import-drop {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  min-height: 96px;
  margin-bottom: 12px;
  padding: 16px;
  border: 1px dashed var(--kb-line);
  background: color-mix(in srgb, var(--kb-primary) 4%, transparent);
  cursor: pointer;
  text-align: center;
  box-sizing: border-box;
}

.kb-import-drop:hover,
.kb-import-drop.is-drag {
  border-color: var(--kb-primary);
  background: color-mix(in srgb, var(--kb-primary) 10%, transparent);
}

.kb-import-drop strong {
  font-size: 13px;
  font-weight: 600;
  color: var(--kb-ink);
}

.kb-import-drop span {
  font-size: 12px;
  color: var(--kb-mute);
}

.kb-import-drop em {
  margin-top: 4px;
  font-style: normal;
  font-size: 12px;
  color: var(--kb-primary);
}

.kb-import-file {
  position: absolute;
  inset: 0;
  opacity: 0;
  pointer-events: none;
}
</style>
