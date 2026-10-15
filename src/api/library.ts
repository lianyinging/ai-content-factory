import { invoke } from '@tauri-apps/api/core'
import type {
  LibraryGroup,
  LibraryGroupListResult,
  LibraryRow,
  Platform,
} from '@/types/item'

/** groupId: undefined/null=全部；''=未分组；其它=指定组 */
export async function libraryList(
  platform: Platform,
  groupId?: string | null,
): Promise<LibraryRow[]> {
  const gid =
    groupId === undefined || groupId === null
      ? null
      : typeof groupId === 'string'
        ? groupId
        : null
  return invoke<LibraryRow[]>('library_list', {
    platform,
    groupId: gid,
  })
}

export async function libraryDelete(platform: Platform, itemId: string) {
  return invoke<{ ok: boolean; deleted: number }>('library_delete', {
    platform,
    itemId,
  })
}

export async function libraryGroupList(platform: Platform) {
  return invoke<LibraryGroupListResult>('library_group_list', { platform })
}

export async function libraryGroupCreate(platform: Platform, name: string) {
  return invoke<LibraryGroup>('library_group_create', { platform, name })
}

export async function libraryGroupRename(platform: Platform, id: string, name: string) {
  return invoke<void>('library_group_rename', { platform, id, name })
}

export async function libraryGroupDelete(platform: Platform, id: string) {
  return invoke<{ ok: boolean; deleted: number }>('library_group_delete', {
    platform,
    id,
  })
}

export async function libraryGroupReorder(platform: Platform, ids: string[]) {
  return invoke<void>('library_group_reorder', { platform, ids })
}

export async function libraryMoveToGroup(
  platform: Platform,
  itemIds: string[],
  groupId: string,
) {
  return invoke<{ ok: boolean; moved: number }>('library_move_to_group', {
    platform,
    itemIds,
    groupId,
  })
}

export async function librarySetAnalysis(opts: {
  platform: Platform
  scope: 'item' | 'group'
  id: string
  analysisJson: string
}) {
  return invoke<void>('library_set_analysis', {
    platform: opts.platform,
    scope: opts.scope,
    id: opts.id,
    analysisJson: opts.analysisJson,
  })
}
