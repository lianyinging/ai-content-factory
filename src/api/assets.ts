import { convertFileSrc, invoke } from '@tauri-apps/api/core'

export type MediaAsset = {
  id: string
  kind: string
  fileName: string
  relPath: string
  absPath: string
  mime: string
  sizeBytes: number
  source: string
  prompt: string
  model: string
  draftId: string
  platform: string
  folderId: string
  sortOrder: number
  createdAt: string
}

export type MediaFolder = {
  id: string
  name: string
  parentId: string
  sortOrder: number
  createdAt: string
}

export async function assetDirInfo() {
  return invoke<{ root: string; images: string; videos: string }>('asset_dir_info')
}

export async function assetList(opts?: {
  kind?: 'image' | 'video' | 'all'
  query?: string
  folderId?: string | null
  limit?: number
  offset?: number
}) {
  return invoke<MediaAsset[]>('asset_list', {
    kind: opts?.kind && opts.kind !== 'all' ? opts.kind : null,
    query: opts?.query || null,
    folderId: opts?.folderId === undefined ? null : opts.folderId,
    limit: opts?.limit ?? 100,
    offset: opts?.offset ?? 0,
  })
}

export async function assetGet(id: string) {
  return invoke<MediaAsset | null>('asset_get', { id })
}

export async function assetIngest(body: {
  sourceUrl?: string
  bytesBase64?: string
  kind?: 'image' | 'video'
  mime?: string
  source?: string
  prompt?: string
  model?: string
  draftId?: string
  platform?: string
  fileName?: string
  folderId?: string
}) {
  return invoke<MediaAsset>('asset_ingest', { body })
}

export async function assetDelete(id: string) {
  return invoke<{ ok: boolean; deleted: number }>('asset_delete', { id })
}

export async function assetRename(id: string, name: string) {
  return invoke<MediaAsset>('asset_rename', { id, name })
}

export async function assetReveal(id: string) {
  return invoke<{ ok: boolean; path: string }>('asset_reveal', { id })
}

export async function assetMove(ids: string[], folderId?: string | null) {
  return invoke<{ ok: boolean; moved: number }>('asset_move', {
    ids,
    folderId: folderId ?? '',
  })
}

export async function assetReorder(ids: string[]) {
  return invoke<{ ok: boolean; updated: number }>('asset_reorder', { ids })
}

export async function mediaFolderReorder(ids: string[]) {
  return invoke<{ ok: boolean; updated: number }>('media_folder_reorder', { ids })
}

export async function mediaFolderMove(id: string, parentId?: string | null) {
  return invoke<MediaFolder>('media_folder_move', {
    id,
    parentId: parentId ?? '',
  })
}

export async function mediaFolderList(parentId?: string | null, query?: string | null) {
  return invoke<MediaFolder[]>('media_folder_list', {
    parentId: parentId ?? '',
    query: query?.trim() || null,
  })
}

export async function mediaFolderCreate(name: string, parentId?: string | null) {
  return invoke<MediaFolder>('media_folder_create', {
    name,
    parentId: parentId ?? '',
  })
}

export async function mediaFolderRename(id: string, name: string) {
  return invoke<MediaFolder>('media_folder_rename', { id, name })
}

export async function mediaFolderDelete(id: string) {
  return invoke<{ ok: boolean; deleted: number }>('media_folder_delete', { id })
}

export async function mediaFolderGet(id: string) {
  return invoke<MediaFolder | null>('media_folder_get', { id })
}

/** Convert local absolute path / http / data URL into something <img> can show. */
export function mediaDisplaySrc(pathOrUrl: string): string {
  const s = String(pathOrUrl || '').trim()
  if (!s) return ''
  if (/^(https?:|data:|asset:|blob:)/i.test(s)) return s
  try {
    return convertFileSrc(s)
  } catch {
    return s
  }
}

export function formatBytes(n: number) {
  if (!Number.isFinite(n) || n <= 0) return '—'
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / 1024 / 1024).toFixed(1)} MB`
}
