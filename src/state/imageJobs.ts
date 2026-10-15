import { computed, ref } from 'vue'
import { imageJobList, type ImageJob } from '@/api/imageJobs'

const jobs = ref<ImageJob[]>([])
let pollTimer: ReturnType<typeof setInterval> | null = null
let pollOpts: { draftId?: string; folderId?: string | null } = {}
const listeners = new Set<() => void>()

export function useImageJobsStore() {
  const hasActive = computed(() =>
    jobs.value.some((j) => j.status === 'queued' || j.status === 'running'),
  )

  async function refresh(opts?: { draftId?: string; folderId?: string | null }) {
    if (opts) pollOpts = { ...opts }
    const prevActive = jobs.value.some((j) => j.status === 'queued' || j.status === 'running')
    try {
      jobs.value = await imageJobList({
        draftId: pollOpts.draftId,
        folderId: pollOpts.folderId,
        limit: 200,
      })
    } catch {
      /* ignore transient */
    }
    const nowActive = jobs.value.some((j) => j.status === 'queued' || j.status === 'running')
    if (prevActive && !nowActive) {
      listeners.forEach((fn) => fn())
    }
    syncPolling()
  }

  function onSettled(fn: () => void) {
    listeners.add(fn)
    return () => listeners.delete(fn)
  }

  function syncPolling() {
    const need = jobs.value.some((j) => j.status === 'queued' || j.status === 'running')
    if (need && !pollTimer) {
      pollTimer = setInterval(() => {
        void refresh()
      }, 2500)
    } else if (!need && pollTimer) {
      clearInterval(pollTimer)
      pollTimer = null
    }
  }

  function stopPolling() {
    if (pollTimer) {
      clearInterval(pollTimer)
      pollTimer = null
    }
  }

  function jobsForDraft(draftId: string) {
    return jobs.value.filter((j) => j.draftId === draftId)
  }

  function upsertLocal(job: ImageJob) {
    const i = jobs.value.findIndex((j) => j.id === job.id)
    if (i >= 0) jobs.value = jobs.value.map((j, idx) => (idx === i ? job : j))
    else jobs.value = [job, ...jobs.value]
    syncPolling()
  }

  function removeLocal(id: string) {
    jobs.value = jobs.value.filter((j) => j.id !== id)
    syncPolling()
  }

  return {
    jobs,
    hasActive,
    refresh,
    stopPolling,
    jobsForDraft,
    upsertLocal,
    removeLocal,
    onSettled,
  }
}
