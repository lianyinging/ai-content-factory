import { computed, ref, type Ref } from 'vue'
import { ElMessage, ElNotification } from 'element-plus'
import { aiViralAnalyze, type ViralAnalyzeItem } from '@/api/ai'
import { librarySetAnalysis } from '@/api/library'
import type { Platform, ViralAnalysis } from '@/types/item'
import router from '@/router'

export type AnalyzeJobStatus = 'queued' | 'running' | 'done' | 'error'

export type AnalyzeJob = {
  id: string
  platform: Platform
  scope: 'item' | 'group'
  targetId: string
  title: string
  items: ViralAnalyzeItem[]
  status: AnalyzeJobStatus
  result?: ViralAnalysis
  error?: string
  createdAt: number
  finishedAt?: number
}

export type PendingOpen = {
  platform: Platform
  scope: 'item' | 'group'
  targetId: string
  title: string
  result: ViralAnalysis | null
  analyzedAt: string
  error?: string
}

type Ctl = {
  jobs: Ref<AnalyzeJob[]>
  pendingOpen: Ref<PendingOpen | null>
  pumping: boolean
  seq: number
}

const CTL_KEY = '__libraryAnalyzeCtl_v1__'

function ctl(): Ctl {
  const g = globalThis as unknown as Record<string, Ctl>
  if (!g[CTL_KEY]) {
    g[CTL_KEY] = {
      jobs: ref<AnalyzeJob[]>([]),
      pendingOpen: ref<PendingOpen | null>(null),
      pumping: false,
      seq: 0,
    }
  }
  return g[CTL_KEY]
}

function jobKey(platform: string, scope: string, targetId: string) {
  return `${platform}:${scope}:${targetId}`
}

function libraryPath(platform: Platform) {
  return platform === 'douyin' ? '/douyin/library' : '/xhs/library'
}

function markPendingOpen(job: AnalyzeJob) {
  const c = ctl()
  c.pendingOpen.value = {
    platform: job.platform,
    scope: job.scope,
    targetId: job.targetId,
    title: job.title,
    result: job.result || null,
    analyzedAt:
      job.finishedAt != null
        ? new Date(job.finishedAt).toISOString().slice(0, 19).replace('T', ' ')
        : '',
    error: job.error,
  }
}

function notifyDone(job: AnalyzeJob) {
  const ok = job.status === 'done'
  ElNotification({
    title: ok ? '爆款分析完成' : '爆款分析失败',
    message: ok
      ? `「${job.title}」已完成，点击查看结果`
      : `「${job.title}」失败：${job.error || '未知错误'}`,
    type: ok ? 'success' : 'error',
    duration: 8000,
    onClick: () => {
      markPendingOpen(job)
      const path = libraryPath(job.platform)
      if (router.currentRoute.value.path !== path) {
        void router.push(path)
      }
    },
  })
}

async function runOne(job: AnalyzeJob) {
  const c = ctl()
  job.status = 'running'
  c.jobs.value = [...c.jobs.value]

  try {
    const result = await aiViralAnalyze({
      scope: job.scope,
      platform: job.platform,
      items: job.items,
    })
    await librarySetAnalysis({
      platform: job.platform,
      scope: job.scope,
      id: job.targetId,
      analysisJson: JSON.stringify(result),
    })
    job.result = result
    job.status = 'done'
    job.finishedAt = Date.now()
  } catch (e) {
    job.status = 'error'
    job.error = e instanceof Error ? e.message : String(e)
    job.finishedAt = Date.now()
  }

  c.jobs.value = [...c.jobs.value]
  notifyDone(job)

  // 保留最近完成任务一段时间，便于 UI 同步；清理过旧的
  const keep = c.jobs.value.filter((j) => {
    if (j.status === 'queued' || j.status === 'running') return true
    return (j.finishedAt || 0) > Date.now() - 10 * 60 * 1000
  })
  c.jobs.value = keep.slice(-30)
}

async function pump() {
  const c = ctl()
  if (c.pumping) return
  c.pumping = true
  try {
    for (;;) {
      const next = c.jobs.value.find((j) => j.status === 'queued')
      if (!next) break
      await runOne(next)
    }
  } finally {
    c.pumping = false
  }
}

export function useLibraryAnalyzeStore() {
  const c = ctl()

  const hasActive = computed(() =>
    c.jobs.value.some((j) => j.status === 'queued' || j.status === 'running'),
  )

  const activeTitle = computed(() => {
    const j = c.jobs.value.find((x) => x.status === 'running')
      || c.jobs.value.find((x) => x.status === 'queued')
    return j?.title || ''
  })

  function isBusy(platform: Platform, scope: 'item' | 'group', targetId: string) {
    const key = jobKey(platform, scope, targetId)
    return c.jobs.value.some(
      (j) =>
        jobKey(j.platform, j.scope, j.targetId) === key &&
        (j.status === 'queued' || j.status === 'running'),
    )
  }

  function findJob(platform: Platform, scope: 'item' | 'group', targetId: string) {
    const key = jobKey(platform, scope, targetId)
    return (
      c.jobs.value.find((j) => jobKey(j.platform, j.scope, j.targetId) === key) || null
    )
  }

  function enqueue(input: {
    platform: Platform
    scope: 'item' | 'group'
    targetId: string
    title: string
    items: ViralAnalyzeItem[]
  }): { ok: boolean; reason?: string; jobId?: string } {
    if (!input.items.length) {
      return { ok: false, reason: '没有可分析的内容' }
    }
    if (isBusy(input.platform, input.scope, input.targetId)) {
      return { ok: false, reason: '该目标正在分析中' }
    }

    const job: AnalyzeJob = {
      id: `la_${Date.now()}_${++c.seq}`,
      platform: input.platform,
      scope: input.scope,
      targetId: input.targetId,
      title: input.title,
      items: input.items,
      status: 'queued',
      createdAt: Date.now(),
    }
    c.jobs.value = [...c.jobs.value, job]
    ElMessage.success('已开始后台分析，可继续浏览其他页面')
    void pump()
    return { ok: true, jobId: job.id }
  }

  function takePendingOpen(platform?: Platform): PendingOpen | null {
    const p = c.pendingOpen.value
    if (!p) return null
    if (platform && p.platform !== platform) return null
    c.pendingOpen.value = null
    return p
  }

  function peekPendingOpen() {
    return c.pendingOpen.value
  }

  return {
    jobs: c.jobs,
    pendingOpen: c.pendingOpen,
    hasActive,
    activeTitle,
    isBusy,
    findJob,
    enqueue,
    takePendingOpen,
    peekPendingOpen,
    markPendingOpen,
  }
}
