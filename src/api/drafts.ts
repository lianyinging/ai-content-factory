import { invoke } from '@tauri-apps/api/core'
import type { Platform } from '@/types/item'

export type DraftRow = {
  id: string
  platform: string
  title: string
  body: string
  tagsJson: string
  imageUrlsJson: string
  sourceItemId: string
  status: string
  updatedAt: string
  createdAt: string
}

export async function draftList(platform: Platform) {
  return invoke<DraftRow[]>('draft_list', { platform })
}

export async function draftUpsert(body: {
  id?: string
  platform: Platform
  title?: string
  body?: string
  tagsJson?: string
  imageUrlsJson?: string
  sourceItemId?: string
  status?: string
}) {
  return invoke<DraftRow>('draft_upsert', { body })
}

export async function draftDelete(id: string) {
  return invoke('draft_delete', { id })
}

export async function libraryToDrafts(platform: Platform, itemIds: string[]) {
  return invoke<{ ok: boolean; created: number }>('library_to_drafts', { platform, itemIds })
}

export function parseTags(json: string): string[] {
  try {
    const v = JSON.parse(json || '[]')
    return Array.isArray(v) ? v.map(String) : []
  } catch {
    return []
  }
}

export function parseImages(json: string): string[] {
  try {
    const v = JSON.parse(json || '[]')
    return Array.isArray(v) ? v.map(String).filter(Boolean) : []
  } catch {
    return []
  }
}

export type PublishLogRow = {
  id: string
  platform: string
  draftId: string
  title: string
  action: string
  ok: boolean
  message: string
  stepsJson: string
  profileId: string
  createdAt: string
}

export async function publishLogList(platform: Platform, limit = 100) {
  return invoke<PublishLogRow[]>('publish_log_list', { platform, limit })
}

export async function publishLogAdd(body: {
  platform: Platform
  draftId?: string
  title?: string
  action: string
  ok: boolean
  message?: string
  stepsJson?: string
  profileId?: string
}) {
  return invoke<PublishLogRow>('publish_log_add', { body })
}

export async function publishLogClear(platform: Platform) {
  return invoke<{ ok: boolean; deleted: number }>('publish_log_clear', { platform })
}

export function parseLogSteps(json: string): string[] {
  try {
    const v = JSON.parse(json || '[]')
    return Array.isArray(v) ? v.map(String) : []
  } catch {
    return []
  }
}
