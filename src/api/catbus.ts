import { h } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { CatbusEnvelope, ContentItem, DoctorInfo, LibraryRow, Platform } from '@/types/item'
import { extractItems } from '@/utils/normalize'

export async function catbusDoctor(): Promise<DoctorInfo> {
  return invoke<DoctorInfo>('catbus_doctor')
}

export async function catbusRun(
  args: string[],
  opts?: { stdin?: string; timeoutSecs?: number },
): Promise<CatbusEnvelope> {
  return invoke<CatbusEnvelope>('catbus_run', {
    req: {
      args,
      stdin: opts?.stdin ?? null,
      timeoutSecs: opts?.timeoutSecs ?? 180,
    },
  })
}

function envelopeError(env: CatbusEnvelope): string {
  const err = env.error
  if (!err) return '请求失败'
  const parts = [err.message || err.code, err.hint].filter(Boolean)
  return parts.join(' · ') || '请求失败'
}

export type CatbusRiskKind = 'captcha' | 'rate_limit' | 'blocked' | ''

export type CatbusErrorInfo = {
  message: string
  hint: string
  code: string
  verifyUrl: string
  riskKind: CatbusRiskKind
  fullText: string
}

function sanitizeUrl(raw: string): string {
  return raw.trim().replace(/[，。；;,.]+$/g, '').replace(/[)\]】》"']+$/g, '')
}

function pickUrlFromText(text: string): string {
  if (!text) return ''
  const m =
    text.match(/https?:\/\/[^\s"'<>\\]+website-login\/captcha[^\s"'<>\\]*/i) ||
    text.match(/https?:\/\/www\.xiaohongshu\.com\/[^\s"'<>\\]+/i) ||
    text.match(/https?:\/\/[^\s"'<>\\]+/i)
  return m?.[0] ? sanitizeUrl(m[0]) : ''
}

function asDetailObject(detail: unknown): Record<string, unknown> | null {
  if (!detail) return null
  if (typeof detail === 'string') {
    try {
      const parsed = JSON.parse(detail)
      return parsed && typeof parsed === 'object' ? (parsed as Record<string, unknown>) : null
    } catch {
      return null
    }
  }
  if (typeof detail === 'object') return detail as Record<string, unknown>
  return null
}

function pickRiskKind(detail: unknown): CatbusRiskKind {
  const d = asDetailObject(detail)
  const kind = String(d?.kind ?? '').toLowerCase()
  if (kind === 'captcha' || kind === 'rate_limit' || kind === 'blocked') return kind
  return ''
}

function pickUrlFromDetail(detail: unknown): string {
  if (!detail) return ''
  if (typeof detail === 'string') {
    return pickUrlFromText(detail) || pickUrlFromDetail(asDetailObject(detail))
  }
  const d = asDetailObject(detail)
  if (!d) return ''
  for (const k of ['verify_url', 'verifyUrl', 'url', 'link']) {
    const v = d[k]
    if (typeof v === 'string' && /^https?:\/\//i.test(v.trim())) return sanitizeUrl(v)
  }
  for (const nest of [d.detail, d.data]) {
    const u = pickUrlFromDetail(nest)
    if (u) return u
  }
  return pickUrlFromText(JSON.stringify(d))
}

/**
 * 小红书人机验证链接：官方 FAQ 写明在 `error.hint` 里；
 * 部分版本也可能放在 detail.verify_url。优先 hint。
 * @see https://github.com/cv-cat/catbus/blob/master/docs/guide/faq.md#%E9%81%87%E5%88%B0-risk_control-%E6%80%8E%E4%B9%88%E5%8A%9E
 */
export function extractVerifyUrl(env: CatbusEnvelope | null | undefined): string {
  const err = env?.error
  if (!err) return ''
  return pickUrlFromText(String(err.hint || '')) || pickUrlFromDetail(err.detail)
}

export function parseCatbusError(
  source: CatbusEnvelope | unknown,
  fallback = '请求失败',
): CatbusErrorInfo {
  if (source && typeof source === 'object' && 'error' in (source as object)) {
    const env = source as CatbusEnvelope
    const err = env.error
    const message = String(err?.message || err?.code || fallback)
    const hint = String(err?.hint || '')
    const code = String(err?.code || '')
    const riskKind = pickRiskKind(err?.detail)
    const verifyUrl =
      extractVerifyUrl(env) || pickUrlFromText(`${hint}\n${message}\n${JSON.stringify(err?.detail ?? '')}`)
    const fullText = [message, hint].filter(Boolean).join(' · ') || fallback
    return { message, hint, code, verifyUrl, riskKind, fullText }
  }
  if (source instanceof Error) {
    const message = source.message || fallback
    const verifyUrl = pickUrlFromText(message)
    return { message, hint: '', code: '', verifyUrl, riskKind: '', fullText: message }
  }
  const text = source == null ? fallback : String(source)
  return {
    message: text,
    hint: '',
    code: '',
    verifyUrl: pickUrlFromText(text),
    riskKind: '',
    fullText: text || fallback,
  }
}

async function copyText(text: string, okMsg = '已复制到剪贴板') {
  try {
    await navigator.clipboard.writeText(text)
    ElMessage.success(okMsg)
    return true
  } catch {
    try {
      const ta = document.createElement('textarea')
      ta.value = text
      ta.style.position = 'fixed'
      ta.style.left = '-9999px'
      document.body.appendChild(ta)
      ta.select()
      document.execCommand('copy')
      document.body.removeChild(ta)
      ElMessage.success(okMsg)
      return true
    } catch {
      ElMessage.warning('自动复制失败，请手动选中文字复制')
      return false
    }
  }
}

function riskGuideLines(kind: CatbusRiskKind, hasLink: boolean): string[] {
  if (hasLink || kind === 'captcha') {
    return [
      '小红书人机验证（HTTP 461/471）：请在已登录同一账号的浏览器中打开下方链接完成验证，再重试。',
      '不要连续重试——请求越紧凑越容易触发，严重时会封号；不验证时有时几分钟自愈，也可能持续数小时。',
    ]
  }
  if (kind === 'rate_limit') {
    return [
      '平台限流（RISK_CONTROL / rate_limit）。请停一段时间再试，不要连续重试。',
      '同步/翻页请拉开间隔，避免并发同一平台；大批量抓取优先减小数量。',
    ]
  }
  if (kind === 'blocked') {
    return [
      '账号或请求被平台拦截（RISK_CONTROL / blocked）。请停一段时间再试，不要连续重试。',
      '若刚扫码登录后触发，可改用浏览器 Cookie 导入登录态。',
    ]
  }
  return [
    '遇到 RISK_CONTROL：不要连续重试，停一段时间再试。',
    '若 hint 中有验证链接，请在已登录同一账号的浏览器中打开完成验证后再重试。',
  ]
}

/** 展示 catbus 错误；RISK_CONTROL / 人机验证按 FAQ 引导，验证链接支持一键复制 */
export async function reportCatbusError(
  source: CatbusEnvelope | unknown,
  fallback = '请求失败',
): Promise<void> {
  const info = parseCatbusError(source, fallback)
  const isRisk =
    Boolean(info.verifyUrl) ||
    Boolean(info.riskKind) ||
    /RISK_CONTROL|人机验证|461|471|captcha|rate_limit|blocked/i.test(
      `${info.code} ${info.message} ${info.hint}`,
    )

  if (!isRisk) {
    ElMessage.error(info.fullText)
    return
  }

  // FAQ：验证链接在 error.hint
  const link = info.verifyUrl || pickUrlFromText(info.hint) || pickUrlFromText(info.fullText)
  const copyPayload = link || info.hint || info.fullText
  const guides = riskGuideLines(info.riskKind, Boolean(link))
  const title =
    link || info.riskKind === 'captcha'
      ? '需要完成小红书人机验证'
      : info.riskKind === 'rate_limit'
        ? '请求过于频繁（限流）'
        : info.riskKind === 'blocked'
          ? '请求被平台拦截'
          : '遇到平台风控（RISK_CONTROL）'

  try {
    await ElMessageBox.confirm(
      h('div', { class: 'catbus-verify-dialog', style: 'text-align: left' }, [
        h('p', { style: 'margin: 0 0 8px; line-height: 1.55; color: #303133' }, info.message),
        ...guides.map((line) =>
          h(
            'p',
            { style: 'margin: 0 0 8px; color: #909399; font-size: 12px; line-height: 1.55' },
            line,
          ),
        ),
        link
          ? h(
              'code',
              {
                style:
                  'display: block; padding: 10px 12px; border-radius: 8px; background: #f5f7fa; color: #303133; font-size: 12px; line-height: 1.5; word-break: break-all; white-space: pre-wrap; margin: 4px 0 12px;',
              },
              link,
            )
          : info.hint
            ? h(
                'p',
                {
                  style:
                    'margin: 4px 0 12px; padding: 10px 12px; border-radius: 8px; background: #f5f7fa; color: #606266; font-size: 12px; line-height: 1.5; word-break: break-all;',
                },
                info.hint,
              )
            : null,
        h(
          'button',
          {
            type: 'button',
            style:
              'display: inline-flex; align-items: center; justify-content: center; height: 32px; padding: 0 14px; border: none; border-radius: 6px; background: var(--el-color-primary, #409eff); color: #fff; font-size: 13px; cursor: pointer;',
            onClick: (ev: Event) => {
              ev.preventDefault()
              ev.stopPropagation()
              void copyText(
                copyPayload,
                link ? '验证链接已复制，请到浏览器打开完成验证' : '提示内容已复制',
              )
            },
          },
          link ? '复制验证链接' : '复制提示内容',
        ),
      ]),
      title,
      {
        confirmButtonText: link ? '复制并关闭' : '我知道了',
        cancelButtonText: '关闭',
        showCancelButton: true,
        distinguishCancelAndClose: true,
        closeOnClickModal: false,
        autofocus: false,
      },
    )
    await copyText(
      copyPayload,
      link ? '验证链接已复制，请到浏览器打开完成验证' : '提示内容已复制',
    )
  } catch {
    /* 用户点关闭 */
  }
}

/** envelope 失败则弹错并返回 true */
export async function reportIfCatbusFailed(
  env: CatbusEnvelope,
  fallback = '请求失败',
): Promise<boolean> {
  if (env.ok !== false && !env.error) return false
  await reportCatbusError(env, fallback)
  return true
}

export async function authList(platform: Platform): Promise<CatbusEnvelope> {
  return catbusRun([platform, 'auth', 'list'], { timeoutSecs: 60 })
}

export async function authStatus(platform: Platform, account = 'default'): Promise<CatbusEnvelope> {
  return catbusRun([platform, 'auth', 'status', '-a', account], { timeoutSecs: 90 })
}

export async function authLogout(platform: Platform, account = 'default'): Promise<CatbusEnvelope> {
  return catbusRun([platform, 'auth', 'logout', '-a', account], { timeoutSecs: 60 })
}

export async function authLoginCookie(
  platform: Platform,
  cookie: string,
  account = 'default',
): Promise<CatbusEnvelope> {
  return catbusRun(
    [platform, 'auth', 'login', '--method', 'cookie', '-a', account, '--cookie', '-'],
    { stdin: cookie, timeoutSecs: 120 },
  )
}

export async function qrStart(platform: Platform, account = 'default') {
  return invoke<{
    ok: boolean
    status: string
    platform: string
    account: string
    hasQr: boolean
  }>('catbus_qr_start', { platform, account })
}

export async function qrStatus(platform: Platform) {
  return invoke<{
    ok: boolean
    status: string
    platform: string
    account: string | null
    error?: string
    envelope?: CatbusEnvelope
    hasQr: boolean
  }>('catbus_qr_status', { platform })
}

export async function qrImage(platform: Platform) {
  return invoke<{
    ok: boolean
    platform: string
    mime?: string
    base64?: string
  }>('catbus_qr_image', { platform })
}

export interface SearchParams {
  query: string
  /** 不传则只取服务端一页；传了会由 catbus 自动翻到凑满 N 条 */
  limit?: number
  /** 上一页 envelope.page.cursor，续翻时原样回传 */
  cursor?: string | null
  sort?: string
  type?: string
  time?: string
  length?: string
  range?: string
  account?: string
}

export interface SearchPage {
  cursor: string | null
  hasMore: boolean
}

function parseSearchPage(envelope: CatbusEnvelope): SearchPage {
  const raw = envelope.page
  if (!raw || typeof raw !== 'object') return { cursor: null, hasMore: false }
  const r = raw as Record<string, unknown>
  const cursor =
    r.cursor != null && String(r.cursor).trim() !== '' ? String(r.cursor) : null
  const hasMore = Boolean(r.has_more) && cursor != null
  return { cursor, hasMore }
}

export async function itemSearch(
  platform: Platform,
  params: SearchParams,
): Promise<{
  ok: boolean
  items: ContentItem[]
  error?: string
  envelope: CatbusEnvelope
  page: SearchPage
}> {
  const q = params.query.trim()
  if (!q) {
    return {
      ok: false,
      items: [],
      error: '请输入关键词',
      envelope: { ok: false },
      page: { cursor: null, hasMore: false },
    }
  }
  const args = [platform, 'item', 'search', q]
  // 不传 --limit：只取一页，靠 --cursor 翻下一页
  if (params.limit != null) args.push('--limit', String(params.limit))
  if (params.cursor) args.push('--cursor', String(params.cursor))
  if (params.sort && params.sort !== 'general') args.push('--sort', params.sort)
  if (params.type && params.type !== 'all') args.push('--type', params.type)
  if (params.time && params.time !== 'all') args.push('--time', params.time)
  if (params.length && params.length !== 'all') args.push('--length', params.length)
  if (params.range && params.range !== 'all') args.push('--range', params.range)
  if (params.account?.trim()) args.push('-a', params.account.trim())

  const envelope = await catbusRun(args, { timeoutSecs: 180 })
  if (!envelope.ok) {
    return {
      ok: false,
      items: [],
      error: envelopeError(envelope),
      envelope,
      page: { cursor: null, hasMore: false },
    }
  }
  const items = extractItems(envelope, platform)
  return { ok: true, items, envelope, page: parseSearchPage(envelope) }
}

export async function librarySave(platform: Platform, items: ContentItem[]) {
  return invoke<{ ok: boolean; saved: number }>('library_save_items', {
    platform,
    items: items.map((it) => ({
      id: it.id,
      title: it.title,
      desc: it.desc,
      coverUrl: it.coverUrl,
      imageUrls: it.imageUrls?.length
        ? it.imageUrls
        : it.coverUrl
          ? [it.coverUrl]
          : [],
      authorName: it.authorName,
      likes: it.likes,
      comments: it.comments,
      collects: it.collects,
      shares: it.shares,
      url: it.url,
      publishedAt: it.publishedAt,
      noteType: it.noteType,
      raw: it.raw,
    })),
  })
}

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

export {
  libraryGroupList,
  libraryGroupCreate,
  libraryGroupRename,
  libraryGroupDelete,
  libraryGroupReorder,
  libraryMoveToGroup,
  librarySetAnalysis,
} from '@/api/library'

export { envelopeError }
