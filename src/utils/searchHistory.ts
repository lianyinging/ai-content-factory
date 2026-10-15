import type { Platform } from '@/types/item'

const PREFIX = 'ai-factory.recentKeywords.'
const MAX = 12

function key(platform: Platform) {
  return `${PREFIX}${platform}`
}

export function loadRecentKeywords(platform: Platform): string[] {
  try {
    const raw = localStorage.getItem(key(platform))
    if (!raw) return []
    const parsed = JSON.parse(raw) as unknown
    if (!Array.isArray(parsed)) return []
    return parsed
      .map((x) => String(x || '').trim())
      .filter(Boolean)
      .slice(0, MAX)
  } catch {
    return []
  }
}

export function saveRecentKeyword(platform: Platform, keyword: string): string[] {
  const kw = keyword.trim()
  if (!kw) return loadRecentKeywords(platform)
  const prev = loadRecentKeywords(platform).filter((x) => x !== kw)
  const next = [kw, ...prev].slice(0, MAX)
  localStorage.setItem(key(platform), JSON.stringify(next))
  return next
}

export function removeRecentKeyword(platform: Platform, keyword: string): string[] {
  const next = loadRecentKeywords(platform).filter((x) => x !== keyword)
  localStorage.setItem(key(platform), JSON.stringify(next))
  return next
}

export function clearRecentKeywords(platform: Platform): void {
  localStorage.removeItem(key(platform))
}
