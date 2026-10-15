<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  MODALITY_OPTIONS,
  PROVIDER_PRESETS,
  canBeAgentDefault,
  deleteLlmProvider,
  fetchLlmBalance,
  isImageModelName,
  listLlmProviders,
  modalityLabel,
  providerSupportsBalance,
  setDefaultLlmProvider,
  testLlmProvider,
  upsertLlmProvider,
  type LlmModality,
  type LlmProvider,
} from '@/llm/llmProviders'

const providers = ref<LlmProvider[]>([])
const loading = ref(false)
const saving = ref(false)
const testingId = ref('')
const balanceById = ref<Record<string, string>>({})
const balanceLoading = ref<Record<string, boolean>>({})
const dialogVisible = ref(false)
const editingId = ref('')

const form = ref({
  name: '',
  provider: 'deepseek',
  modality: 'text' as LlmModality,
  baseUrl: '',
  apiKey: '',
  defaultModel: '',
  timeoutSec: 120,
  enabled: true,
  isDefault: false,
})

const preset = computed(() => PROVIDER_PRESETS.find((item) => item.id === form.value.provider))
const modelOptions = computed(() => preset.value?.models || [])

function resetForm(provider?: LlmProvider | null) {
  if (!provider) {
    const deepseek = PROVIDER_PRESETS[0]
    const modality = deepseek.defaultModality || 'text'
    const hasChatDefault = providers.value.some(
      (item) => canBeAgentDefault(item.modality) && item.isDefault,
    )
    form.value = {
      name: deepseek.label,
      provider: deepseek.id,
      modality,
      baseUrl: deepseek.baseUrl,
      apiKey: '',
      defaultModel: deepseek.defaultModel,
      timeoutSec: 120,
      enabled: true,
      isDefault: canBeAgentDefault(modality) && !hasChatDefault,
    }
    return
  }
  form.value = {
    name: provider.name,
    provider: provider.provider,
    modality: (provider.modality as LlmModality) || 'text',
    baseUrl: provider.baseUrl,
    apiKey: '',
    defaultModel: provider.defaultModel,
    timeoutSec: provider.timeoutSec,
    enabled: provider.enabled,
    isDefault: provider.isDefault,
  }
}

function formatBalanceText(result: Awaited<ReturnType<typeof fetchLlmBalance>>): string {
  if (result.ok && result.items.length) {
    return result.items.map((item) => `${item.currency} ${item.totalBalance}`).join(' · ')
  }
  return result.message || '—'
}

async function loadBalances(rows: LlmProvider[]) {
  const targets = rows.filter((row) => providerSupportsBalance(row.provider))
  if (!targets.length) return

  const patch: Record<string, string> = {}
  const loadingMap: Record<string, boolean> = { ...balanceLoading.value }

  for (const row of targets) {
    if (!row.hasApiKey) {
      patch[row.id] = '未配置'
      delete loadingMap[row.id]
      continue
    }
    loadingMap[row.id] = true
  }
  balanceLoading.value = loadingMap
  if (Object.keys(patch).length) {
    balanceById.value = { ...balanceById.value, ...patch }
  }

  await Promise.all(
    targets
      .filter((row) => row.hasApiKey)
      .map(async (row) => {
        let text = '—'
        try {
          text = formatBalanceText(await fetchLlmBalance(row.id))
        } catch (err) {
          text = err instanceof Error ? err.message : String(err)
        } finally {
          const nextLoading = { ...balanceLoading.value }
          delete nextLoading[row.id]
          balanceLoading.value = nextLoading
          balanceById.value = { ...balanceById.value, [row.id]: text }
        }
      }),
  )
}

async function refresh() {
  loading.value = true
  try {
    providers.value = await listLlmProviders()
    void loadBalances(providers.value)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
  }
}

function openCreate() {
  editingId.value = ''
  resetForm(null)
  dialogVisible.value = true
}

function openEdit(row: LlmProvider) {
  editingId.value = row.id
  resetForm(row)
  dialogVisible.value = true
}

function onPresetChange(id: string) {
  const p = PROVIDER_PRESETS.find((x) => x.id === id)
  if (!p) return
  form.value.name = p.label
  form.value.baseUrl = p.baseUrl
  form.value.defaultModel = p.defaultModel
  if (p.defaultModality) form.value.modality = p.defaultModality
  else if (isImageModelName(p.defaultModel)) form.value.modality = 'image'
}

function onModelChange(model: string) {
  if (isImageModelName(model)) form.value.modality = 'image'
}

async function save() {
  saving.value = true
  try {
    await upsertLlmProvider({
      id: editingId.value || undefined,
      name: form.value.name,
      provider: form.value.provider,
      modality: form.value.modality,
      baseUrl: form.value.baseUrl,
      apiKey: form.value.apiKey || undefined,
      defaultModel: form.value.defaultModel,
      timeoutSec: form.value.timeoutSec,
      enabled: form.value.enabled,
      isDefault: form.value.isDefault,
    })
    ElMessage.success('已保存')
    dialogVisible.value = false
    await refresh()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    saving.value = false
  }
}

async function onDelete(row: LlmProvider) {
  try {
    await ElMessageBox.confirm(`删除「${row.name}」？`, '确认', { type: 'warning' })
  } catch {
    return
  }
  try {
    await deleteLlmProvider(row.id)
    ElMessage.success('已删除')
    await refresh()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function onSetDefault(row: LlmProvider) {
  try {
    await setDefaultLlmProvider(row.id)
    ElMessage.success('已设为默认')
    await refresh()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function onTest(row: LlmProvider) {
  testingId.value = row.id
  try {
    const r = await testLlmProvider(row.id)
    if (r.ok) ElMessage.success(r.message)
    else ElMessage.error(r.message)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    testingId.value = ''
  }
}

onMounted(refresh)
</script>

<template>
  <section class="mf-ops__panel model-config" v-loading="loading">
    <div class="mf-ops__panel-head">
      <strong>大模型配置</strong>
      <div class="mf-ops__panel-head-actions">
        <el-button size="small" @click="refresh">刷新</el-button>
        <el-button type="primary" size="small" @click="openCreate">添加供应商</el-button>
      </div>
    </div>
    <div class="mf-ops__panel-body">
      <p class="lede">
        配置 OpenAI 兼容接口（DeepSeek / OpenAI 文本与图片 / 通义 / 智谱等）。密钥仅保存在本机；文本默认模型用于草稿与竞品助手，图片模态（如
        gpt-image-2）用于配图生成。
      </p>
      <el-table :data="providers" empty-text="暂无供应商，请先添加">
        <el-table-column prop="name" label="名称" min-width="120" />
        <el-table-column label="模态" width="90">
          <template #default="{ row }">{{ modalityLabel(row.modality) }}</template>
        </el-table-column>
        <el-table-column prop="defaultModel" label="模型" min-width="140" show-overflow-tooltip />
        <el-table-column label="Key" width="100">
          <template #default="{ row }">
            {{ row.hasApiKey ? row.apiKeyHint || '已配置' : '未配置' }}
          </template>
        </el-table-column>
        <el-table-column label="余额" min-width="140" show-overflow-tooltip>
          <template #default="{ row }">
            <template v-if="providerSupportsBalance(row.provider)">
              <span v-if="balanceLoading[row.id]" class="balance-muted">查询中…</span>
              <span v-else class="balance-mono">{{ balanceById[row.id] || '—' }}</span>
            </template>
            <span v-else class="balance-muted">—</span>
          </template>
        </el-table-column>
        <el-table-column label="状态" width="100">
          <template #default="{ row }">
            <el-tag v-if="row.isDefault" size="small" type="success">默认</el-tag>
            <el-tag v-else size="small" :type="row.enabled ? 'info' : 'info'" effect="plain">
              {{ row.enabled ? '启用' : '停用' }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="280" fixed="right">
          <template #default="{ row }">
            <el-button link type="primary" @click="openEdit(row)">编辑</el-button>
            <el-button
              link
              type="primary"
              :loading="testingId === row.id"
              @click="onTest(row)"
            >
              测试
            </el-button>
            <el-button
              v-if="providerSupportsBalance(row.provider)"
              link
              :loading="balanceLoading[row.id]"
              :disabled="!row.hasApiKey"
              @click="loadBalances([row])"
            >
              查余额
            </el-button>
            <el-button
              v-if="canBeAgentDefault(row.modality) && !row.isDefault"
              link
              @click="onSetDefault(row)"
            >
              设默认
            </el-button>
            <el-button link type="danger" @click="onDelete(row)">删除</el-button>
          </template>
        </el-table-column>
      </el-table>
    </div>

    <el-dialog
      v-model="dialogVisible"
      :title="editingId ? '编辑供应商' : '添加供应商'"
      width="520px"
      destroy-on-close
    >
      <el-form label-width="100px" size="small">
        <el-form-item label="厂商预设">
          <el-select v-model="form.provider" style="width: 100%" @change="onPresetChange">
            <el-option
              v-for="p in PROVIDER_PRESETS"
              :key="p.id"
              :label="p.label"
              :value="p.id"
            />
          </el-select>
        </el-form-item>
        <el-form-item label="名称" required>
          <el-input v-model="form.name" />
        </el-form-item>
        <el-form-item label="模态">
          <el-select v-model="form.modality" style="width: 100%">
            <el-option
              v-for="m in MODALITY_OPTIONS"
              :key="m.value"
              :label="m.label"
              :value="m.value"
            />
          </el-select>
        </el-form-item>
        <el-form-item label="Base URL" required>
          <el-input v-model="form.baseUrl" placeholder="https://api.example.com/v1" />
        </el-form-item>
        <el-form-item label="API Key" :required="!editingId">
          <el-input
            v-model="form.apiKey"
            type="password"
            show-password
            :placeholder="editingId ? '留空则保留原 Key' : '必填'"
          />
        </el-form-item>
        <el-form-item label="默认模型" required>
          <el-select
            v-if="modelOptions.length"
            v-model="form.defaultModel"
            filterable
            allow-create
            style="width: 100%"
            @change="onModelChange"
          >
            <el-option v-for="m in modelOptions" :key="m" :label="m" :value="m" />
          </el-select>
          <el-input v-else v-model="form.defaultModel" @change="onModelChange(form.defaultModel)" />
          <p v-if="form.modality === 'image'" class="field-hint">
            图片模型走 Images API（/v1/images/generations），请勿设为文本默认。
          </p>
        </el-form-item>
        <el-form-item label="超时(秒)">
          <el-input-number v-model="form.timeoutSec" :min="5" :max="600" />
        </el-form-item>
        <el-form-item label="启用">
          <el-switch v-model="form.enabled" />
        </el-form-item>
        <el-form-item v-if="canBeAgentDefault(form.modality)" label="设为默认">
          <el-switch v-model="form.isDefault" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button size="small" @click="dialogVisible = false">取消</el-button>
        <el-button type="primary" size="small" :loading="saving" @click="save">保存</el-button>
      </template>
    </el-dialog>
  </section>
</template>

<style scoped>
.model-config {
  flex: none;
  max-width: 960px;
}
.lede {
  margin: 0 0 14px;
  color: var(--muted);
  font-size: 13px;
}
.balance-mono {
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  font-size: 12px;
}
.balance-muted {
  color: var(--muted);
  font-size: 12px;
}
.field-hint {
  margin: 6px 0 0;
  font-size: 12px;
  color: var(--muted);
  line-height: 1.4;
}
</style>
