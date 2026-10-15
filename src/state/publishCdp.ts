import { reactive } from 'vue'
import { listen } from '@tauri-apps/api/event'
import type { Platform } from '@/types/item'
import { draftUpsert, parseTags, publishLogAdd } from '@/api/drafts'
import {
  browserCdpStatusAll,
  launchBrowserCdp,
  publishDouyinNote,
  publishXhsNote,
} from '@/api/cdp'

export type PublishCdpAction = 'fill' | 'saveDraft' | 'publish'

export type PublishLiveTone = 'info' | 'ok' | 'warn' | 'err'

export type PublishLiveLine = {
  id: number
  time: string
  tag: string
  message: string
  tone: PublishLiveTone
}

export type PublishCdpJob = {
  platform: Platform
  draftId: string
  draftTitle: string
  action: PublishCdpAction
  profileId: string
  status: 'running' | 'done' | 'error'
  steps: string[]
  message: string
  published: boolean
  savedDraft: boolean
  nextStatus: string
  startedAt: number
  finishedAt?: number
  /** 结果是否已在发布中心展示过（避免重复 toast） */
  seen?: boolean
  /** 滚动日志（真实 CDP 进度 + 结果摘要） */
  liveLines: PublishLiveLine[]
  /** 是否已收到过 CDP 实时进度 */
  liveGotRealProgress?: boolean
  /** 结果摘要是否已追加到 liveLines */
  liveRealAppended?: boolean
}

type StartInput = {
  platform: Platform
  draftId: string
  draftTitle: string
  draftBody: string
  tagsJson: string
  imageUrlsJson: string
  sourceItemId: string
  images: string[]
  profileId: string
  action: PublishCdpAction
}

const jobs = reactive<Record<string, PublishCdpJob>>({})
const liveSeqByKey: Record<string, number> = {}
let cdpProgressListening = false

const LIVE_LINE_CAP = 80

function fmtLiveClock(ts = Date.now()) {
  const d = new Date(ts)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
}

export function publishJobKey(platform: string, draftId: string) {
  return `${platform}:${draftId}`
}

export function getPublishJob(platform: string, draftId: string) {
  return jobs[publishJobKey(platform, draftId)] || null
}

export function isPublishJobRunning(platform: string, draftId?: string) {
  if (!draftId) {
    return Object.values(jobs).some((j) => j.platform === platform && j.status === 'running')
  }
  return getPublishJob(platform, draftId)?.status === 'running'
}

export function markPublishJobSeen(platform: string, draftId: string) {
  const job = getPublishJob(platform, draftId)
  if (job && job.status !== 'running') job.seen = true
}

export function clearPublishJob(platform: string, draftId: string) {
  const key = publishJobKey(platform, draftId)
  delete jobs[key]
  delete liveSeqByKey[key]
}

export function appendPublishLiveLine(
  platform: string,
  draftId: string,
  input: { tag: string; message: string; tone?: PublishLiveTone },
) {
  const key = publishJobKey(platform, draftId)
  const job = jobs[key]
  if (!job) return
  liveSeqByKey[key] = (liveSeqByKey[key] || 0) + 1
  if (!job.liveLines) job.liveLines = []
  job.liveLines.push({
    id: liveSeqByKey[key],
    time: fmtLiveClock(),
    tag: input.tag,
    message: input.message,
    tone: input.tone || 'info',
  })
  if (job.liveLines.length > LIVE_LINE_CAP) {
    job.liveLines.splice(0, job.liveLines.length - LIVE_LINE_CAP)
  }
}

function clearLiveLines(key: string) {
  const job = jobs[key]
  if (!job) return
  job.liveLines = []
  job.liveGotRealProgress = false
  job.liveRealAppended = false
  liveSeqByKey[key] = 0
}

function findRunningJobByDraftId(draftId: string) {
  const id = String(draftId || '')
  if (!id) return null
  return (
    Object.values(jobs).find((j) => j.draftId === id && j.status === 'running') ||
    Object.values(jobs).find((j) => j.draftId === id) ||
    null
  )
}

function applyRealtimeStep(draftId: string, step: string) {
  const job = findRunningJobByDraftId(draftId)
  if (!job || job.status !== 'running') return
  job.liveGotRealProgress = true
  const { label, ok } = formatPublishStepLabel(step)
  appendPublishLiveLine(job.platform, job.draftId, {
    tag: 'CDP',
    message: label,
    tone: ok ? 'ok' : 'err',
  })
}

async function ensureCdpProgressListener() {
  if (cdpProgressListening) return
  cdpProgressListening = true
  try {
    await listen<{
      step: string
      draftId: string
      action?: string
    }>('cdp-progress', (ev) => {
      const step = String(ev.payload?.step || '')
      const draftId = String(ev.payload?.draftId || '')
      if (!step) return
      applyRealtimeStep(draftId, step)
    })
  } catch {
    cdpProgressListening = false
  }
}

void ensureCdpProgressListener()

/** 把机器 step 码译成滚动日志文案 */
export function formatPublishStepLabel(raw: string): { label: string; ok: boolean } {
  const s = String(raw || '').trim()
  if (!s) return { label: '—', ok: true }
  if (s === 'reused_publish_tab' || s === 'reused_upload_tab') {
    return { label: '复用已有发布页', ok: true }
  }
  if (s.startsWith('reused_creator_tab')) {
    return { label: '复用创作平台标签并进入发布页', ok: true }
  }
  if (
    s === 'opened_publish' ||
    s === 'opened_publish_new_tab' ||
    s === 'opened_upload' ||
    s === 'opened_upload_new_tab'
  ) {
    return { label: '打开发布页', ok: true }
  }
  if (s.startsWith('creator_tab:')) {
    const m = s.match(/^creator_tab:([^:]+):(\w+)/)
    const mode = m?.[1] || '发布模式'
    const ok = m?.[2] !== 'false'
    return { label: ok ? `已切换「${mode}」` : `未找到「${mode}」Tab`, ok }
  }
  if (s.startsWith('uploaded:')) {
    return { label: `已上传 ${s.slice('uploaded:'.length)} 个媒体`, ok: true }
  }
  if (s === 'filled_title') return { label: '已填写标题', ok: true }
  if (s === 'filled_content') return { label: '已填写正文', ok: true }
  if (s.startsWith('click_publish:')) {
    const ok = !s.endsWith(':false')
    return { label: ok ? '已点击发布' : '未点到发布按钮', ok }
  }
  if (s.startsWith('click_save_draft:')) {
    const ok = !s.endsWith(':false')
    return { label: ok ? '已点击「暂存离开」' : '未点到暂存按钮', ok }
  }
  if (s.startsWith('confirm:')) {
    return { label: !s.endsWith(':false') ? '已确认发布' : '无二次确认弹窗', ok: true }
  }
  if (s.startsWith('image_tab:')) {
    const ok = !s.endsWith(':false')
    return { label: ok ? '已切换图文入口' : '未切换图文入口', ok }
  }
  if (s.startsWith('topic:') && s.endsWith(':ok')) {
    const name = s.slice('topic:'.length, -':ok'.length)
    return { label: `话题已选 #${name}`, ok: true }
  }
  if (s.startsWith('topic:') && (s.includes(':fail') || s.includes(':err'))) {
    const name = s.split(':')[1] || ''
    return { label: `话题未选上 #${name}`, ok: false }
  }
  if (s.startsWith('topics_filled:')) {
    return { label: `话题完成 ${s.slice('topics_filled:'.length)}`, ok: true }
  }
  if (s.startsWith('recommend_topics:')) {
    const rest = s.slice('recommend_topics:'.length)
    if (rest === 'none' || rest === 'empty' || rest === '0') {
      return { label: '无推荐话题可点', ok: true }
    }
    return { label: `已点推荐话题 ${rest.replace(/\|/g, ' · ')}`, ok: true }
  }
  if (s.startsWith('recommend_topic:fail')) {
    return { label: '推荐话题点击失败', ok: false }
  }
  if (s === 'cdp_script_start') return { label: 'CDP 脚本已启动', ok: true }
  if (s.startsWith('images_ready:')) {
    return { label: `配图已就绪 ${s.slice('images_ready:'.length)} 张`, ok: true }
  }
  if (s.startsWith('reused_creator_tab:navigated')) {
    return { label: '复用创作平台标签并进入发布页', ok: true }
  }
  return { label: s, ok: true }
}

function appendRealResultToLive(job: PublishCdpJob) {
  if (job.liveRealAppended) return
  job.liveRealAppended = true
  const { platform, draftId } = job
  // 若运行中已流式推过真实步骤，结束时只补结果摘要，避免重复刷屏
  if (!job.liveGotRealProgress) {
    appendPublishLiveLine(platform, draftId, {
      tag: '结果',
      message: '—— 以下为真实执行回执 ——',
      tone: 'info',
    })
    for (const raw of job.steps || []) {
      const { label, ok } = formatPublishStepLabel(raw)
      appendPublishLiveLine(platform, draftId, {
        tag: '步骤',
        message: label,
        tone: ok ? 'ok' : 'err',
      })
    }
  } else {
    appendPublishLiveLine(platform, draftId, {
      tag: '结果',
      message: '—— 执行结束 ——',
      tone: 'info',
    })
  }
  appendPublishLiveLine(platform, draftId, {
    tag: job.status === 'error' ? '失败' : '完成',
    message: job.message || (job.status === 'error' ? '自动化失败' : '完成'),
    tone: job.status === 'error' ? 'err' : 'ok',
  })
}

async function ensureProfileOnline(profileId: string) {
  const sum = await browserCdpStatusAll()
  const st = (sum.profiles || []).find((p) => p.profileId === profileId)
  if (st?.cdpOnline) return st
  await launchBrowserCdp(profileId)
  const again = await browserCdpStatusAll()
  const st2 = (again.profiles || []).find((p) => p.profileId === profileId)
  if (!st2?.cdpOnline) throw new Error('CDP 仍离线，请到账号矩阵检查浏览器')
  return st2
}

export function startPublishCdpJob(input: StartInput): { ok: boolean; reason?: string } {
  const key = publishJobKey(input.platform, input.draftId)
  if (jobs[key]?.status === 'running') {
    return { ok: false, reason: '该草稿正在自动化中' }
  }
  if (input.action === 'saveDraft' && input.platform !== 'xhs') {
    return { ok: false, reason: '暂存草稿目前仅支持小红书' }
  }

  jobs[key] = {
    platform: input.platform,
    draftId: input.draftId,
    draftTitle: input.draftTitle,
    action: input.action,
    profileId: input.profileId,
    status: 'running',
    steps: [],
    message: '',
    published: false,
    savedDraft: false,
    nextStatus: 'filled',
    startedAt: Date.now(),
    seen: false,
    liveLines: [],
    liveGotRealProgress: false,
    liveRealAppended: false,
  }

  clearLiveLines(key)
  void ensureCdpProgressListener()
  const actLabel =
    input.action === 'publish'
      ? '填表并发布'
      : input.action === 'saveDraft'
        ? '保存到草稿'
        : '仅填表'
  appendPublishLiveLine(input.platform, input.draftId, {
    tag: '启动',
    message: `后台任务已启动 · ${actLabel}`,
    tone: 'info',
  })

  void (async () => {
    const job = jobs[key]
    if (!job) return
    try {
      appendPublishLiveLine(input.platform, input.draftId, {
        tag: 'CDP',
        message: '检查 Profile 在线…',
        tone: 'info',
      })
      const st = await ensureProfileOnline(input.profileId)
      appendPublishLiveLine(input.platform, input.draftId, {
        tag: 'CDP',
        message: `已连接调试端口 ${st.debugPort}`,
        tone: 'ok',
      })
      const media = input.images
      const looksVideo = media.some((u) => /\.(mp4|mov|webm|m4v)(\?|$)/i.test(u))
      const looksAudio = media.some((u) => /\.(mp3|m4a|aac|wav)(\?|$)/i.test(u))
      const contentType = looksVideo ? 'video' : looksAudio ? 'podcast' : 'image'
      const payload = {
        debugPort: st.debugPort,
        title: input.draftTitle,
        content: input.draftBody,
        tags: parseTags(input.tagsJson),
        imageUrls: media,
        contentType,
        mode:
          contentType === 'video' ? '上传视频' : contentType === 'podcast' ? '发播客' : '上传图文',
        action: input.action,
        publish: input.action === 'publish',
        saveDraft: input.action === 'saveDraft',
        draftId: input.draftId,
      }
      const r =
        input.platform === 'xhs'
          ? await publishXhsNote(payload)
          : await publishDouyinNote(payload)

      const steps = Array.isArray(r.steps) ? (r.steps as string[]) : []
      const message = String(r.message || '完成')
      const published = Boolean(r.published) || input.action === 'publish'
      const savedDraft = Boolean(r.savedDraft) || input.action === 'saveDraft'
      const nextStatus = published ? 'published' : savedDraft ? 'platform_draft' : 'filled'

      await draftUpsert({
        id: input.draftId,
        platform: input.platform,
        title: input.draftTitle,
        body: input.draftBody,
        tagsJson: input.tagsJson,
        imageUrlsJson: input.imageUrlsJson,
        sourceItemId: input.sourceItemId,
        status: nextStatus,
      })
      try {
        await publishLogAdd({
          platform: input.platform,
          draftId: input.draftId,
          title: input.draftTitle,
          action: input.action,
          ok: true,
          message,
          stepsJson: JSON.stringify(steps),
          profileId: input.profileId,
        })
      } catch {
        /* ignore */
      }

      Object.assign(job, {
        status: 'done' as const,
        steps,
        message,
        published,
        savedDraft,
        nextStatus,
        finishedAt: Date.now(),
        seen: false,
      })
      appendRealResultToLive(job)
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      try {
        await publishLogAdd({
          platform: input.platform,
          draftId: input.draftId,
          title: input.draftTitle,
          action: input.action,
          ok: false,
          message,
          stepsJson: JSON.stringify(job.steps || []),
          profileId: input.profileId,
        })
      } catch {
        /* ignore */
      }
      Object.assign(job, {
        status: 'error' as const,
        message,
        finishedAt: Date.now(),
        seen: false,
      })
      appendRealResultToLive(job)
    }
  })()

  return { ok: true }
}
