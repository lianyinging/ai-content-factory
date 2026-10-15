import { invoke } from '@tauri-apps/api/core'

export type BrowserCandidate = { kind: string; label: string; path: string }

export type BrowserConfig = {
  browserExecutable: string
  browserKind: string
  userDataDir: string
  debugPort: number
  autoLaunch: boolean
}

export type BrowserProfile = {
  id: string
  name: string
  platforms: string
  userDataDir: string
  debugPort: number
  sessionJson: string
}

export type PlatformSession = {
  loggedIn?: boolean
  nickname?: string
  avatarUrl?: string
  redId?: string
  following?: string
  followers?: string
  likesCollect?: string
  updatedAt?: number
}

export type BrowserStatus = {
  profileId: string
  profileName: string
  configured: boolean
  running: boolean
  cdpOnline: boolean
  debugPort: number
  browserExecutable: string
  userDataDir: string
  browserKind: string
  autoLaunch: boolean
  platforms: string
  sessionJson: string
  message: string
  browserVersion?: string | null
  webSocketDebuggerUrl?: string | null
}

export type BrowserStatusSummary = {
  configured: boolean
  onlineCount: number
  totalCount: number
  profiles: BrowserStatus[]
  message: string
  running: boolean
  cdpOnline: boolean
  debugPort: number
  browserExecutable: string
  userDataDir: string
  browserKind: string
  autoLaunch: boolean
}

export function parseProfileSession(sessionJson?: string | null): {
  xhs?: PlatformSession
  douyin?: PlatformSession
} {
  try {
    const raw = JSON.parse(sessionJson || '{}') as Record<string, PlatformSession>
    return { xhs: raw.xhs, douyin: raw.douyin }
  } catch {
    return {}
  }
}

export async function detectBrowsers() {
  return invoke<BrowserCandidate[]>('detect_browsers')
}

export async function getBrowserConfig() {
  return invoke<BrowserConfig>('get_browser_config')
}

export async function saveBrowserConfig(body: BrowserConfig) {
  return invoke<BrowserConfig>('save_browser_config', { body })
}

export async function listBrowserProfiles() {
  return invoke<BrowserProfile[]>('list_browser_profiles')
}

export async function upsertBrowserProfile(body: {
  id?: string
  name: string
  platforms?: string
  userDataDir?: string
  debugPort?: number
}) {
  return invoke<BrowserProfile>('upsert_browser_profile', { body })
}

export async function deleteBrowserProfile(profileId: string) {
  await invoke('delete_browser_profile', { profileId })
}

export async function updateBrowserProfileSession(profileId: string, sessionJson: string) {
  return invoke<BrowserProfile>('update_browser_profile_session', {
    body: { profileId, sessionJson },
  })
}

export async function browserCdpStatus(profileId?: string) {
  return invoke<BrowserStatus>('browser_cdp_status', { profileId: profileId ?? null })
}

export async function browserCdpStatusAll() {
  return invoke<BrowserStatusSummary>('browser_cdp_status_all')
}

export async function launchBrowserCdp(profileId?: string) {
  return invoke<BrowserStatus>('launch_browser_cdp', { profileId: profileId ?? null })
}

export async function stopBrowserCdp(profileId?: string) {
  return invoke<BrowserStatus>('stop_browser_cdp', { profileId: profileId ?? null })
}

export async function loginBrowserPlatform(platform: string, profileId?: string) {
  return invoke<BrowserStatus>('login_browser_platform', {
    profileId: profileId ?? null,
    platform,
  })
}

export async function openBrowserUrl(url: string, profileId?: string) {
  return invoke<BrowserStatus>('open_browser_url', {
    profileId: profileId ?? null,
    url,
  })
}

type NodeJobResult = {
  ok: boolean
  result?: Record<string, unknown> | null
  error?: string | null
}

async function cdpJob(action: string, payload: Record<string, unknown>, timeoutSecs = 120) {
  const res = await invoke<NodeJobResult>('cdp_run', {
    req: { action, payload, timeoutSecs },
  })
  if (!res.ok) throw new Error(res.error || String(res.result?.error || `CDP ${action} 失败`))
  return res.result || {}
}

export async function probeBrowserSession(debugPort: number, platform: 'xhs' | 'douyin') {
  return cdpJob('probe-session', { debugPort, platform }, 90)
}

export async function browserSmoke(debugPort: number, url = 'https://example.com') {
  return cdpJob('smoke', { debugPort, url }, 60)
}

export async function publishXhsNote(payload: {
  debugPort: number
  title: string
  content: string
  tags?: string[]
  imageUrls?: string[]
  imagePaths?: string[]
  /** 上传视频 | 上传图文 | 写长文 | 发播客；也可传 video/image/long/podcast */
  mode?: string
  contentType?: string
  /** fill | saveDraft | publish；saveDraft 点「暂存离开」 */
  action?: 'fill' | 'saveDraft' | 'publish'
  publish?: boolean
  saveDraft?: boolean
  draftId?: string
}) {
  return cdpJob('publish-xhs-note', payload as unknown as Record<string, unknown>, 300)
}

export async function publishDouyinNote(payload: {
  debugPort: number
  title: string
  content: string
  tags?: string[]
  imageUrls?: string[]
  imagePaths?: string[]
  publish?: boolean
  draftId?: string
}) {
  return cdpJob('publish-douyin-note', payload as unknown as Record<string, unknown>, 300)
}

export type BrowserTabInfo = {
  id: string
  title: string
  url: string
  type: string
}

export type BrowserTabsSnapshot = {
  profileId: string
  profileName: string
  debugPort: number
  cdpOnline: boolean
  browserVersion?: string | null
  tabCount: number
  pageCount: number
  tabs: BrowserTabInfo[]
  message: string
}

export async function listBrowserTabs(profileId?: string) {
  return invoke<BrowserTabsSnapshot>('list_browser_tabs', { profileId: profileId ?? null })
}

export type XhsMessageBadgeProbe = {
  profileId: string
  profileName: string
  debugPort: number
  cdpOnline: boolean
  tabId: string
  tabTitle: string
  tabUrl: string
  spanFound: boolean
  badgeFound: boolean
  countFound: boolean
  count: number | null
  countText: string
  message: string
  probedAt: number
}

export async function probeXhsMessageBadge(profileId?: string, tabId?: string) {
  return invoke<XhsMessageBadgeProbe>('probe_xhs_message_badge', {
    profileId: profileId ?? null,
    tabId: tabId ?? null,
  })
}

export type XhsChatMessage = {
  direction: 'recv' | 'sent' | 'unknown' | string
  text: string
}

export type XhsUnreadConversation = {
  index: number
  badge: number
  preview: string
  name?: string
}

export type XhsUnreadReadResult = {
  profileId: string
  profileName: string
  debugPort: number
  cdpOnline: boolean
  tabId: string
  tabTitle: string
  tabUrl: string
  navBadge: number | null
  clickedNav: boolean
  conversations: XhsUnreadConversation[]
  openedIndex: number | null
  peerName?: string
  messages: XhsChatMessage[]
  replied: boolean
  replyText: string
  steps: { name: string; ok: boolean; detail: string }[]
  message: string
  probedAt: number
}

export async function readXhsUnreadMessages(profileId?: string, tabId?: string) {
  return invoke<XhsUnreadReadResult>('read_xhs_unread_messages', {
    profileId: profileId ?? null,
    tabId: tabId ?? null,
  })
}

export type XhsImUnreadConv = {
  index: number
  badge: number
  preview: string
  name?: string
  active: boolean
}

export type XhsImStateProbe = {
  profileId: string
  profileName: string
  debugPort: number
  cdpOnline: boolean
  tabId: string
  tabTitle: string
  tabUrl: string
  inIm: boolean
  inChat: boolean
  peerName?: string
  activeConvIndex?: number
  activeConvName?: string
  messageCount: number
  messages: XhsChatMessage[]
  lastDirection: string
  lastText: string
  fingerprint: string
  unreadConversations: XhsImUnreadConv[]
  message: string
  probedAt: number
}

export type XhsImActionResult = {
  ok: boolean
  replyText: string
  message: string
  peerName?: string
  index?: number | null
  tabId: string
  probedAt: number
}

export async function probeXhsImState(profileId?: string) {
  return invoke<XhsImStateProbe>('probe_xhs_im_state', { profileId: profileId ?? null })
}

export type XhsConvListItem = {
  index: number
  name: string
  badge: number
  preview: string
  active: boolean
}

export type XhsConvListProbe = {
  profileId: string
  profileName: string
  debugPort: number
  cdpOnline: boolean
  tabId: string
  inIm: boolean
  conversations: XhsConvListItem[]
  message: string
  probedAt: number
}

export async function probeXhsConvList(profileId?: string) {
  return invoke<XhsConvListProbe>('probe_xhs_conv_list', { profileId: profileId ?? null })
}

export type XhsFilterTag = {
  text: string
  dataHpKind: string
  ariaHidden: boolean
  opacity: string
  visible: boolean
  x: number
  y: number
  w: number
  h: number
}

export type XhsFilterPanelProbe = {
  ok: boolean
  profileId: string
  tabId: string
  filterFound: boolean
  panelFound: boolean
  tags: XhsFilterTag[]
  message: string
  probedAt: number
  steps: { name: string; ok: boolean; detail: string }[]
}

export async function probeXhsFilterPanel(profileId?: string) {
  return invoke<XhsFilterPanelProbe>('probe_xhs_filter_panel', {
    profileId: profileId ?? null,
  })
}

export type XhsMatchedComment = {
  index: number
  text: string
  matchedKeyword: string
  hasReplyIcon: boolean
}

export type XhsCommentMatchProbe = {
  ok: boolean
  profileId: string
  tabId: string
  scrollerFound: boolean
  commentsFound: boolean
  scrollerSelector: string
  scrollerX: number
  scrollerY: number
  totalComments: number
  matched: XhsMatchedComment[]
  scrollsDone: number
  message: string
  probedAt: number
  steps: { name: string; ok: boolean; detail: string }[]
}

/** 当前已打开的笔记详情：鼠标定位评论滑动区 → 滚轮加载 → 按关键词匹配（不发送） */
export async function probeXhsCommentMatch(opts: {
  profileId?: string
  commentKeyword: string
  maxScrolls?: number
}) {
  return invoke<XhsCommentMatchProbe>('probe_xhs_comment_match', {
    body: {
      profileId: opts.profileId ?? null,
      commentKeyword: opts.commentKeyword,
      maxScrolls: opts.maxScrolls ?? 5,
    },
  })
}

export type XhsCustomReplyProbe = {
  ok: boolean
  profileId: string
  tabId: string
  scrollerFound: boolean
  matched: number
  sent: number
  scrolls: number
  message: string
  probedAt: number
  steps: { name: string; ok: boolean; detail: string }[]
}

/** 当前已打开笔记：匹配评论 → 点回复 → 填入自定义文案并发送（探测完整回复链路） */
export async function probeXhsCustomReply(opts: {
  profileId?: string
  commentKeyword: string
  replyText: string
  maxScrolls?: number
  maxReplies?: number
}) {
  return invoke<XhsCustomReplyProbe>('probe_xhs_custom_reply', {
    body: {
      profileId: opts.profileId ?? null,
      commentKeyword: opts.commentKeyword,
      replyText: opts.replyText,
      maxScrolls: opts.maxScrolls ?? 5,
      maxReplies: opts.maxReplies ?? 1,
    },
  })
}

export type XhsAcqRunResult = {
  ok: boolean
  profileId: string
  tabId: string
  keyword: string
  commentKeyword: string
  sortLabel: string
  notesOpened: number
  commentsMatched: number
  repliesSent: number
  steps: { name: string; ok: boolean; detail: string }[]
  message: string
  probedAt: number
}

/** 立即返回；长任务在 Rust 后台线程跑，进度/结束见事件 */
export type XhsAcqStartResult = {
  ok: boolean
  profileId: string
  tabId: string
  message: string
}

export async function xhsAcqRun(opts: {
  profileId?: string
  /** 笔记搜索关键词 */
  keyword: string
  /** 评论匹配关键词（可多词） */
  commentKeyword: string
  sortLabel?: string
  replyText: string
  maxNotes?: number
  maxRepliesPerNote?: number
  /** 单篇内对 .note-content 下滑加载更多评论次数，默认 5 */
  maxCommentScrolls?: number
}) {
  return invoke<XhsAcqStartResult>('xhs_acq_run', {
    body: {
      profileId: opts.profileId ?? null,
      keyword: opts.keyword,
      commentKeyword: opts.commentKeyword,
      sortLabel: opts.sortLabel || '最新',
      replyText: opts.replyText,
      maxNotes: opts.maxNotes ?? 5,
      maxRepliesPerNote: opts.maxRepliesPerNote ?? 1,
      maxCommentScrolls: opts.maxCommentScrolls ?? 5,
    },
  })
}

export async function clickXhsConvItem(profileId: string | undefined, index: number) {
  return invoke<XhsImActionResult>('click_xhs_conv_item', {
    profileId: profileId ?? null,
    index,
  })
}

export async function sendXhsChatReply(profileId?: string, text?: string) {
  return invoke<XhsImActionResult>('send_xhs_chat_reply', {
    profileId: profileId ?? null,
    text: text ?? null,
  })
}

export async function setBrowserAiBanner(profileId: string | undefined, visible: boolean) {
  return invoke<XhsImActionResult>('set_browser_ai_banner', {
    profileId: profileId ?? null,
    visible,
  })
}

export async function withAiBrowserBanner<T>(
  profileId: string | undefined,
  fn: () => Promise<T>,
): Promise<T> {
  try {
    await setBrowserAiBanner(profileId, true)
  } catch {
    /* ignore */
  }
  try {
    return await fn()
  } finally {
    try {
      await setBrowserAiBanner(profileId, false)
    } catch {
      /* ignore */
    }
  }
}

/** Probe session for a profile and merge into session_json. */
export async function refreshProfileSession(
  profile: BrowserProfile,
  platforms: Array<'xhs' | 'douyin'> = ['xhs', 'douyin'],
) {
  const prev = parseProfileSession(profile.sessionJson)
  const next: Record<string, PlatformSession> = { ...prev }
  const results: Record<string, unknown> = {}
  for (const platform of platforms) {
    try {
      const r = (await probeBrowserSession(profile.debugPort, platform)) as Record<string, unknown>
      results[platform] = r
      next[platform] = {
        loggedIn: Boolean(r.loggedIn),
        nickname: String(r.nickname || ''),
        avatarUrl: String(r.avatarUrl || ''),
        redId: String(r.redId || ''),
        following: String(r.following || ''),
        followers: String(r.followers || ''),
        likesCollect: String(r.likesCollect || ''),
        updatedAt: Date.now(),
      }
    } catch (e) {
      results[platform] = { error: e instanceof Error ? e.message : String(e) }
    }
  }
  const updated = await updateBrowserProfileSession(profile.id, JSON.stringify(next))
  return { profile: updated, results, session: next }
}
