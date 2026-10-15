import { invoke } from '@tauri-apps/api/core'
import { llmPayload, requireImageLlm } from '@/llm/llmProviders'

export type ImageJob = {
  id: string
  kind: string
  status: string
  prompt: string
  model: string
  size: string
  quality: string
  baseImagePath: string
  draftId: string
  folderId: string
  platform: string
  assetId: string
  absPath: string
  error: string
  createdAt: string
  startedAt: string
  finishedAt: string
}

export async function imageJobStart(input: {
  kind: 'generate' | 'edit'
  prompt: string
  size?: string
  quality?: string
  baseImagePath?: string
  draftId?: string
  folderId?: string
  platform?: string
}) {
  const runtime = await requireImageLlm()
  const timeoutSecs = Math.max(60, runtime.timeoutSec || 180)
  return invoke<ImageJob>('image_job_start', {
    body: {
      kind: input.kind,
      prompt: input.prompt,
      size: input.size || '1024x1024',
      quality: input.quality || 'medium',
      baseImagePath: input.baseImagePath || '',
      draftId: input.draftId || '',
      folderId: input.folderId || '',
      platform: input.platform || '',
      llm: llmPayload(runtime),
      timeoutSecs,
    },
  })
}

export async function imageJobList(opts?: {
  draftId?: string
  folderId?: string | null
  status?: string
  limit?: number
}) {
  return invoke<ImageJob[]>('image_job_list', {
    draftId: opts?.draftId || null,
    folderId: opts?.folderId === undefined ? null : opts.folderId,
    status: opts?.status || null,
    limit: opts?.limit ?? 100,
  })
}

export async function imageJobGet(id: string) {
  return invoke<ImageJob | null>('image_job_get', { id })
}

export async function imageJobDelete(id: string) {
  return invoke<{ ok: boolean; deleted: number }>('image_job_delete', { id })
}

export function jobStatusLabel(status: string) {
  if (status === 'queued') return '排队中'
  if (status === 'running') return '生成中'
  if (status === 'done') return '已完成'
  if (status === 'error') return '失败'
  return status || '—'
}
