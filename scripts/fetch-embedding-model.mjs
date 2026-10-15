#!/usr/bin/env node
/**
 * 下载 BAAI/bge-small-zh-v1.5 到 resources/model/BAAI/bge-small-zh-v1.5（实体文件，无软链）。
 * CI / 本机缺模型时运行：node scripts/fetch-embedding-model.mjs
 */
import { createWriteStream, existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { pipeline } from 'node:stream/promises'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = join(__dirname, '..')
const DEST = join(ROOT, 'resources', 'model', 'BAAI', 'bge-small-zh-v1.5')
const REPO = 'BAAI/bge-small-zh-v1.5'
const BASE = `https://huggingface.co/${REPO}/resolve/main`

const FILES = [
  'config.json',
  'config_sentence_transformers.json',
  'modules.json',
  'sentence_bert_config.json',
  'special_tokens_map.json',
  'tokenizer.json',
  'tokenizer_config.json',
  'vocab.txt',
  'README.md',
  'model.safetensors',
  '1_Pooling/config.json',
]

async function download(url, dest) {
  const res = await fetch(url, { redirect: 'follow' })
  if (!res.ok) throw new Error(`download failed ${res.status}: ${url}`)
  mkdirSync(dirname(dest), { recursive: true })
  await pipeline(res.body, createWriteStream(dest))
}

async function main() {
  const force = process.argv.includes('--force')
  const marker = join(DEST, 'config.json')
  const weight = join(DEST, 'model.safetensors')
  if (!force && existsSync(marker) && existsSync(weight)) {
    console.log(`[fetch-embedding-model] already present: ${DEST}`)
    return
  }
  if (force && existsSync(DEST)) {
    rmSync(DEST, { recursive: true, force: true })
  }
  mkdirSync(DEST, { recursive: true })
  for (const rel of FILES) {
    const url = `${BASE}/${rel}`
    const dest = join(DEST, rel)
    if (!force && existsSync(dest)) {
      console.log(`[skip] ${rel}`)
      continue
    }
    console.log(`[get] ${rel}`)
    await download(url, dest)
  }
  writeFileSync(join(DEST, '.fetched'), `${new Date().toISOString()}\n${REPO}\n`)
  console.log(`[fetch-embedding-model] ok -> ${DEST}`)
}

main().catch((e) => {
  console.error(e)
  process.exit(1)
})
