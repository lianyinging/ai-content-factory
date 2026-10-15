import type { CatbusEnvelope, ContentItem, Platform } from '@/types/item'

function asInt(value: unknown): number {
  if (value == null) return 0
  if (typeof value === 'boolean') return value ? 1 : 0
  if (typeof value === 'number' && Number.isFinite(value)) return Math.trunc(value)
  const text = String(value).trim().replace(/,/g, '')
  if (!text) return 0
  const last = text.slice(-1)
  try {
    if (last === 'w' || last === 'W' || last === '万') return Math.trunc(parseFloat(text.slice(0, -1)) * 10000)
    if (last === 'k' || last === 'K' || last === '千') return Math.trunc(parseFloat(text.slice(0, -1)) * 1000)
    return Math.trunc(parseFloat(text))
  } catch {
    return 0
  }
}

function pickStr(...vals: unknown[]): string {
  for (const v of vals) {
    if (v == null) continue
    const s = String(v).trim()
    if (s) return s
  }
  return ''
}

export function normalizeItem(raw: unknown, platform: Platform): ContentItem | null {
  if (!raw || typeof raw !== 'object') return null
  const r = raw as Record<string, unknown>
  const id = pickStr(r.id, r.aweme_id, r.awemeId, r.item_id, r.noteId)
  if (!id) return null

  const author =
    r.author && typeof r.author === 'object' ? (r.author as Record<string, unknown>) : {}
  const stats = r.stats && typeof r.stats === 'object' ? (r.stats as Record<string, unknown>) : {}

  let cover: unknown = r.cover ?? r.coverUrl ?? r.cover_url
  if (!cover && Array.isArray(r.images) && r.images.length) cover = r.images[0]
  if (cover && typeof cover === 'object') {
    const c = cover as Record<string, unknown>
    cover = c.url ?? c.url_list
    if (Array.isArray(cover) && cover.length) cover = cover[0]
  }

  const imageUrls: string[] = []
  const pushUrl = (v: unknown) => {
    if (typeof v === 'string') {
      const t = v.trim()
      if (t && !imageUrls.includes(t)) imageUrls.push(t)
      return
    }
    if (v && typeof v === 'object') {
      const o = v as Record<string, unknown>
      const u = o.url ?? o.urlDefault ?? o.url_default
      if (typeof u === 'string') {
        const t = u.trim()
        if (t && !imageUrls.includes(t)) imageUrls.push(t)
        return
      }
      if (Array.isArray(o.url_list) && o.url_list.length) {
        const t = String(o.url_list[0] || '').trim()
        if (t && !imageUrls.includes(t)) imageUrls.push(t)
      }
    }
  }
  for (const key of ['imageUrls', 'images', 'image_list', 'imageList']) {
    const arr = r[key]
    if (Array.isArray(arr)) {
      for (const v of arr) pushUrl(v)
    }
  }

  const text = pickStr(r.text, r.desc, r.description, r.content)
  const title = pickStr(r.title, text).slice(0, 200)
  const coverUrl = pickStr(cover)

  return {
    id,
    platform,
    title,
    desc: text || title,
    coverUrl,
    imageUrls: imageUrls.length ? imageUrls : coverUrl ? [coverUrl] : [],
    authorName: pickStr(author.nickname, author.name, r.authorName, r.author_name),
    authorId: pickStr(author.id, author.uid, r.authorId, r.author_id),
    likes: asInt(stats.likes ?? stats.digg_count ?? r.likes ?? r.digg_count),
    comments: asInt(stats.comments ?? stats.comment_count ?? r.comments ?? r.comment_count),
    collects: asInt(stats.collects ?? stats.collect_count ?? r.collects ?? r.collect_count),
    shares: asInt(stats.shares ?? stats.share_count ?? r.shares ?? r.share_count),
    url: pickStr(r.url, r.share_url, r.shareUrl, r.noteUrl),
    publishedAt: pickStr(r.created_at, r.createdAt, r.publishedAt),
    noteType: pickStr(r.kind, r.type, r.noteType) || 'video',
    raw,
  }
}

export function extractItems(envelope: CatbusEnvelope, platform: Platform): ContentItem[] {
  const data = envelope.data
  let items: unknown[] = []
  if (Array.isArray(data)) {
    items = data
  } else if (data && typeof data === 'object') {
    const d = data as Record<string, unknown>
    for (const key of ['items', 'list', 'aweme_list', 'data']) {
      if (Array.isArray(d[key])) {
        items = d[key] as unknown[]
        break
      }
    }
  }
  const out: ContentItem[] = []
  for (const raw of items) {
    const n = normalizeItem(raw, platform)
    if (n) out.push(n)
  }
  return out
}
