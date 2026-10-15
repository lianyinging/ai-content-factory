#!/usr/bin/env node
/**
 * Download official Node.js binary and place it as Tauri externalBin:
 *   src-tauri/binaries/node-<target-triple>[.exe]
 *
 * Usage:
 *   node scripts/fetch-node-sidecar.mjs
 *   node scripts/fetch-node-sidecar.mjs --version 22.22.2
 *   node scripts/fetch-node-sidecar.mjs --triple aarch64-apple-darwin
 */
import {
  chmodSync,
  copyFileSync,
  createWriteStream,
  existsSync,
  mkdirSync,
  renameSync,
  rmSync,
  unlinkSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'
import { pipeline } from 'node:stream/promises'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = join(__dirname, '..')
const BIN_DIR = join(ROOT, 'src-tauri', 'binaries')
const DEFAULT_VERSION = '22.22.2'

function parseArgs(argv) {
  const out = { version: DEFAULT_VERSION, triple: '' }
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i]
    if (a === '--version' && argv[i + 1]) out.version = argv[++i]
    else if (a === '--triple' && argv[i + 1]) out.triple = argv[++i]
  }
  return out
}

function hostTriple() {
  try {
    return execFileSync('rustc', ['--print', 'host-tuple'], { encoding: 'utf8' }).trim()
  } catch {
    try {
      const info = execFileSync('rustc', ['-vV'], { encoding: 'utf8' })
      const m = /host:\s*(\S+)/.exec(info)
      if (m) return m[1]
    } catch {
      /* fall through */
    }
    // Fallback when rustc is unavailable (dev script on CI hosts)
    const arch = process.arch === 'arm64' ? 'aarch64' : 'x86_64'
    if (process.platform === 'darwin') return `${arch}-apple-darwin`
    if (process.platform === 'linux') return `${arch}-unknown-linux-gnu`
    if (process.platform === 'win32') return `${arch}-pc-windows-msvc`
    throw new Error('cannot detect host triple')
  }
}

/** Map rust target triple → node dist platform/arch */
function nodeDistForTriple(triple) {
  const map = {
    'aarch64-apple-darwin': { platform: 'darwin', arch: 'arm64', ext: 'tar.gz' },
    'x86_64-apple-darwin': { platform: 'darwin', arch: 'x64', ext: 'tar.gz' },
    'x86_64-unknown-linux-gnu': { platform: 'linux', arch: 'x64', ext: 'tar.gz' },
    'aarch64-unknown-linux-gnu': { platform: 'linux', arch: 'arm64', ext: 'tar.gz' },
    'x86_64-pc-windows-msvc': { platform: 'win', arch: 'x64', ext: 'zip' },
    'aarch64-pc-windows-msvc': { platform: 'win', arch: 'arm64', ext: 'zip' },
  }
  const hit = map[triple]
  if (!hit) throw new Error(`unsupported triple for Node sidecar: ${triple}`)
  return hit
}

async function download(url, dest) {
  const res = await fetch(url)
  if (!res.ok) throw new Error(`download failed ${res.status}: ${url}`)
  await pipeline(res.body, createWriteStream(dest))
}

/** Windows CI 上 temp 与 workspace 常跨盘，rename 会报 EXDEV */
function moveFile(src, dest) {
  try {
    renameSync(src, dest)
  } catch (e) {
    if (e && (e.code === 'EXDEV' || e.errno === -4037)) {
      copyFileSync(src, dest)
      unlinkSync(src)
      return
    }
    throw e
  }
}

async function main() {
  const { version, triple: tripleArg } = parseArgs(process.argv)
  const triple = tripleArg || hostTriple()
  const dist = nodeDistForTriple(triple)
  const base = `node-v${version}-${dist.platform}-${dist.arch}`
  const url = `https://nodejs.org/dist/v${version}/${base}.${dist.ext}`
  const outName = dist.platform === 'win' ? `node-${triple}.exe` : `node-${triple}`
  const outPath = join(BIN_DIR, outName)

  mkdirSync(BIN_DIR, { recursive: true })
  if (existsSync(outPath)) {
    console.log(`[fetch-node-sidecar] already exists: ${outPath}`)
    return
  }

  const work = join(tmpdir(), `ai-factory-node-${version}-${Date.now()}`)
  mkdirSync(work, { recursive: true })
  const archive = join(work, `${base}.${dist.ext}`)
  console.log(`[fetch-node-sidecar] downloading ${url}`)
  await download(url, archive)

  if (dist.ext === 'tar.gz') {
    execFileSync('tar', ['-xzf', archive, '-C', work], { stdio: 'inherit' })
    const nodeBin = join(work, base, 'bin', 'node')
    if (!existsSync(nodeBin)) throw new Error(`node binary missing in archive: ${nodeBin}`)
    moveFile(nodeBin, outPath)
    chmodSync(outPath, 0o755)
  } else {
    execFileSync('unzip', ['-q', archive, '-d', work], { stdio: 'inherit' })
    const nodeBin = join(work, base, 'node.exe')
    if (!existsSync(nodeBin)) throw new Error(`node.exe missing in archive: ${nodeBin}`)
    moveFile(nodeBin, outPath)
  }

  rmSync(work, { recursive: true, force: true })
  console.log(`[fetch-node-sidecar] wrote ${outPath}`)
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
