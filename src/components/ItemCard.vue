<script setup lang="ts">
import { ref } from 'vue'
import type { ContentItem } from '@/types/item'
import { openUrl } from '@tauri-apps/plugin-opener'
import { librarySave } from '@/api/catbus'
import { competitorAdd } from '@/api/competitors'
import { ElMessage } from 'element-plus'

const props = defineProps<{
  item: ContentItem
}>()

const saving = ref(false)
const monitoring = ref(false)
const coverBroken = ref(false)

async function openLink() {
  if (!props.item.url) return
  try {
    await openUrl(props.item.url)
  } catch {
    window.open(props.item.url, '_blank')
  }
}

async function addToLibrary() {
  saving.value = true
  try {
    await librarySave(props.item.platform, [props.item])
    ElMessage.success('已入库')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    saving.value = false
  }
}

async function addMonitor() {
  if (props.item.platform !== 'xhs') {
    ElMessage.info('抖音竞品监控后续接入')
    return
  }
  if (!props.item.url) {
    ElMessage.warning('缺少笔记链接')
    return
  }
  monitoring.value = true
  try {
    await competitorAdd({
      noteUrl: props.item.url,
      noteId: props.item.id,
      title: props.item.title,
      authorName: props.item.authorName,
      coverUrl: props.item.coverUrl,
      likes: props.item.likes,
      collects: props.item.collects,
      comments: props.item.comments,
      shares: props.item.shares,
    })
    ElMessage.success('已加入竞品监控')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    monitoring.value = false
  }
}

function fmt(n: number) {
  if (n >= 10000) return `${(n / 10000).toFixed(1)}万`
  return String(n ?? 0)
}

function onCoverError() {
  coverBroken.value = true
}
</script>

<template>
  <article class="note-card">
    <div class="note-card__cover">
      <img
        v-if="item.coverUrl && !coverBroken"
        :src="item.coverUrl"
        :alt="item.title"
        loading="lazy"
        referrerpolicy="no-referrer"
        @error="onCoverError"
      />
      <div v-else class="note-card__cover-empty">无封面</div>
    </div>
    <div class="note-card__body">
      <h3 class="note-card__title">{{ item.title || '未命名' }}</h3>
      <p class="note-card__author">{{ item.authorName || '未知作者' }}</p>
      <div class="note-card__metrics">
        <span>赞 {{ fmt(item.likes) }}</span>
        <span>藏 {{ fmt(item.collects) }}</span>
        <span>评 {{ fmt(item.comments) }}</span>
        <span>转 {{ fmt(item.shares) }}</span>
      </div>
      <div class="note-card__actions">
        <button
          type="button"
          class="note-card__btn note-card__btn--primary"
          :disabled="saving"
          @click="addToLibrary"
        >
          {{ saving ? '…' : '入库' }}
        </button>
        <button
          type="button"
          class="note-card__btn"
          :disabled="monitoring || item.platform !== 'xhs'"
          @click="addMonitor"
        >
          {{ monitoring ? '…' : '监控' }}
        </button>
        <button
          type="button"
          class="note-card__btn"
          :disabled="!item.url"
          @click="openLink"
        >
          打开
        </button>
      </div>
    </div>
  </article>
</template>
