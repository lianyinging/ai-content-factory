#!/usr/bin/env node
/**
 * 把 resources 下软链接替换为实体文件/目录，避免 Windows Tauri 打包因 symlink 失败。
 */
import {
  copyFileSync,
  cpSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readdirSync,
  readlinkSync,
  rmSync,
  statSync,
} from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = join(__dirname, '..')
const ROOTS = [
  join(ROOT, 'resources', 'cdp'),
  join(ROOT, 'resources', 'ai'),
  join(ROOT, 'resources', 'catbus'),
  join(ROOT, 'resources', 'model'),
]

function walk(dir, out = []) {
  if (!existsSync(dir)) return out
  let entries
  try {
    entries = readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const ent of entries) {
    const p = join(dir, ent.name)
    let st
    try {
      st = lstatSync(p)
    } catch {
      continue
    }
    if (st.isSymbolicLink()) {
      out.push(p)
      continue
    }
    if (st.isDirectory()) walk(p, out)
  }
  return out
}

function derefOne(linkPath) {
  let target
  try {
    target = readlinkSync(linkPath)
  } catch {
    return false
  }
  const abs = resolve(dirname(linkPath), target)
  if (!existsSync(abs)) {
    // 悬空软链：直接删除，避免打包炸掉
    rmSync(linkPath, { force: true })
    console.warn(`[rm dangling] ${linkPath} -> ${target}`)
    return true
  }
  const targetStat = statSync(abs)
  rmSync(linkPath, { force: true, recursive: true })
  mkdirSync(dirname(linkPath), { recursive: true })
  if (targetStat.isDirectory()) {
    cpSync(abs, linkPath, { recursive: true, dereference: true })
  } else {
    copyFileSync(abs, linkPath)
  }
  console.log(`[deref] ${linkPath}`)
  return true
}

let n = 0
for (const root of ROOTS) {
  const links = walk(root)
  // 从深到浅替换，避免父目录先被处理
  links.sort((a, b) => b.length - a.length)
  for (const link of links) {
    if (derefOne(link)) n += 1
  }
}
console.log(`[deref-resource-symlinks] replaced ${n} symlink(s)`)
