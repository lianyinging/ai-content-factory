import { invoke } from '@tauri-apps/api/core'

export type LlmModality = 'text' | 'image' | 'video' | 'multimodal'

export const MODALITY_OPTIONS: { value: LlmModality; label: string }[] = [
  { value: 'text', label: '文本' },
  { value: 'image', label: '图片' },
  { value: 'video', label: '视频' },
  { value: 'multimodal', label: '多模态' },
]

export function modalityLabel(modality: string): string {
  return MODALITY_OPTIONS.find((item) => item.value === modality)?.label || modality || '文本'
}

export function canBeAgentDefault(modality: string): boolean {
  return modality === 'text' || modality === 'multimodal'
}

export type LlmProvider = {
  id: string
  name: string
  provider: string
  modality: LlmModality | string
  baseUrl: string
  apiKeyHint: string
  hasApiKey: boolean
  defaultModel: string
  extraModels: string
  timeoutSec: number
  enabled: boolean
  isDefault: boolean
  updatedAt: number
}

export type LlmRuntime = {
  id: string
  name: string
  provider: string
  modality: LlmModality | string
  baseUrl: string
  apiKey: string
  model: string
  timeoutSec: number
}

export type UpsertLlmProvider = {
  id?: string
  name: string
  provider: string
  modality?: LlmModality | string
  baseUrl: string
  apiKey?: string
  defaultModel: string
  extraModels?: string
  timeoutSec?: number
  enabled?: boolean
  isDefault?: boolean
}

export type LlmTestResult = {
  ok: boolean
  latencyMs: number
  message: string
}

export type LlmBalanceItem = {
  currency: string
  totalBalance: string
  grantedBalance: string
  toppedUpBalance: string
}

export type LlmBalanceResult = {
  ok: boolean
  supported: boolean
  available: boolean | null
  message: string
  items: LlmBalanceItem[]
}

export type ProviderPreset = {
  id: string
  label: string
  baseUrl: string
  defaultModel: string
  models: string[]
  defaultModality?: LlmModality
  /** Whether this vendor exposes a balance API we can call. */
  supportsBalance?: boolean
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: 'deepseek',
    label: 'DeepSeek',
    // 官方 OpenAI 兼容：https://api-docs.deepseek.com/zh-cn/
    // 余额：GET /user/balance
    baseUrl: 'https://api.deepseek.com',
    defaultModel: 'deepseek-flash',
    models: ['deepseek-flash', 'deepseek-v4-pro'],
    defaultModality: 'text',
    supportsBalance: true,
  },
  {
    id: 'openai',
    label: 'OpenAI',
    baseUrl: 'https://api.openai.com/v1',
    defaultModel: 'gpt-4o-mini',
    models: ['gpt-4o-mini', 'gpt-4o', 'gpt-4.1-mini'],
    defaultModality: 'text',
  },
  {
    id: 'openai_image',
    label: 'OpenAI 图片',
    // Images API：https://api.openai.com/v1/images/generations
    baseUrl: 'https://api.openai.com/v1',
    defaultModel: 'gpt-image-2',
    models: [
      'gpt-image-2',
      'gpt-image-2-2026-04-21',
      'gpt-image-1.5',
      'gpt-image-1',
      'gpt-image-1-mini',
    ],
    defaultModality: 'image',
  },
  {
    // https://flymux.com/docs/ai-api-gateway/openai-api-gateway
    id: 'flymux',
    label: 'FlyMux',
    baseUrl: 'https://api.flymux.com/v1',
    defaultModel: 'gpt-image-2',
    models: ['gpt-image-2', 'gpt-image-1.5', 'gpt-4o-mini', 'gpt-4o', 'deepseek-chat'],
    defaultModality: 'image',
  },
  {
    id: 'qwen',
    label: '通义千问',
    baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    defaultModel: 'qwen-plus',
    models: ['qwen-plus', 'qwen-turbo', 'qwen-max'],
  },
  {
    id: 'zhipu',
    label: '智谱 GLM',
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    defaultModel: 'glm-4-flash',
    models: ['glm-4-flash', 'glm-4', 'glm-4-plus'],
  },
  {
    id: 'custom',
    label: '自定义',
    baseUrl: '',
    defaultModel: '',
    models: [],
  },
]

export function providerSupportsBalance(providerId: string): boolean {
  return Boolean(PROVIDER_PRESETS.find((item) => item.id === providerId)?.supportsBalance)
}

/** Models that only work with Images API (not chat/completions). */
export function isImageModelName(model: string): boolean {
  const m = model.trim().toLowerCase()
  return m.startsWith('gpt-image') || m.startsWith('dall-e')
}

export async function listLlmProviders(): Promise<LlmProvider[]> {
  return invoke<LlmProvider[]>('list_llm_providers')
}

export async function upsertLlmProvider(body: UpsertLlmProvider): Promise<LlmProvider> {
  return invoke<LlmProvider>('upsert_llm_provider', { body })
}

export async function deleteLlmProvider(id: string): Promise<void> {
  await invoke('delete_llm_provider', { id })
}

export async function setDefaultLlmProvider(id: string): Promise<void> {
  await invoke('set_default_llm_provider', { id })
}

export async function getLlmRuntimeByModality(modality: string): Promise<LlmRuntime | null> {
  return invoke<LlmRuntime | null>('get_llm_runtime_by_modality', { modality })
}

export async function testLlmProvider(id: string): Promise<LlmTestResult> {
  return invoke<LlmTestResult>('test_llm_provider', { id })
}

export async function fetchLlmBalance(id: string): Promise<LlmBalanceResult> {
  return invoke<LlmBalanceResult>('fetch_llm_balance', { id })
}

export function llmPayload(runtime: LlmRuntime) {
  return {
    baseUrl: runtime.baseUrl,
    apiKey: runtime.apiKey,
    model: runtime.model,
    timeoutSec: runtime.timeoutSec,
  }
}

export async function requireTextLlm(): Promise<LlmRuntime> {
  const r = await getLlmRuntimeByModality('text')
  if (!r?.apiKey) {
    throw new Error('请先在「系统设置 → 大模型配置」中配置并启用文本模型')
  }
  return r
}

export async function requireImageLlm(): Promise<LlmRuntime> {
  const r = await getLlmRuntimeByModality('image')
  if (!r?.apiKey) {
    throw new Error('请先在「系统设置 → 大模型配置」中添加「OpenAI 图片」并选择 gpt-image-2')
  }
  return r
}
