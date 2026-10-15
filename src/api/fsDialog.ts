import { invoke } from '@tauri-apps/api/core'

/** 系统「另存为」对话框保存文本；取消返回 null */
export async function saveTextWithDialog(input: {
  defaultName: string
  content: string
  filterName: string
  extensions: string[]
}) {
  return invoke<string | null>('save_text_with_dialog', {
    defaultName: input.defaultName,
    content: input.content,
    filterName: input.filterName,
    extensions: input.extensions,
  })
}

/** 读取本地文本文件（拖放路径等） */
export async function readTextFile(path: string) {
  return invoke<string>('read_text_file', { path })
}
