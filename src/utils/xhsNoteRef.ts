/** 小红书详情/评论需要 xsec_token：优先传带 token 的完整 URL */
export function extractNoteId(url: string) {
  const m =
    url.match(/explore\/([a-zA-Z0-9]+)/) ||
    url.match(/discovery\/item\/([a-zA-Z0-9]+)/) ||
    url.match(/item\/([a-zA-Z0-9]+)/) ||
    url.match(/^([0-9a-f]{24})$/i)
  return m?.[1] || ''
}

export function noteRefForCatbus(noteUrl?: string, noteId?: string): string {
  const url = String(noteUrl || '').trim()
  const id = String(noteId || '').trim() || extractNoteId(url)
  if (url && /xsec_token=/i.test(url)) return url
  if (url && /^https?:\/\//i.test(url)) return url
  return id
}
