<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import {
  clickXhsConvItem,
  probeXhsConvList,
  probeXhsImState,
  type XhsConvListItem,
} from '@/api/cdp'
import {
  csMessageList,
  csMessagesSync,
  csSessionList,
  csSessionTouch,
  type CsMessage,
  type CsSession,
} from '@/api/knowledge'

const props = defineProps<{
  modelValue: boolean
  profileId: string
}>()

const emit = defineEmits<{
  'update:modelValue': [boolean]
}>()

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v),
})

const loading = ref(false)
const syncing = ref(false)
const sessions = ref<CsSession[]>([])
const messages = ref<CsMessage[]>([])
const selectedId = ref('')
const liveIndexByPeer = ref<Record<string, number>>({})
const chatScrollEl = ref<HTMLElement | null>(null)

const selected = computed(() => sessions.value.find((s) => s.id === selectedId.value) || null)

function fmtTime(ms: number) {
  if (!ms) return ''
  const d = new Date(ms)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

async function scrollChatBottom() {
  await nextTick()
  const el = chatScrollEl.value
  if (el) el.scrollTop = el.scrollHeight
}

async function loadSessions() {
  if (!props.profileId) {
    sessions.value = []
    return
  }
  // 后端已过滤 message_count>0；前端再兜底，只展示有对话交互的用户
  const rows = await csSessionList(props.profileId)
  sessions.value = rows.filter((s) => (s.messageCount || 0) > 0)
  if (!selectedId.value && sessions.value.length) {
    selectedId.value = sessions.value[0].id
  } else if (selectedId.value && !sessions.value.some((s) => s.id === selectedId.value)) {
    selectedId.value = sessions.value[0]?.id || ''
  }
}

async function loadMessages() {
  if (!selectedId.value) {
    messages.value = []
    return
  }
  messages.value = await csMessageList(selectedId.value)
  await scrollChatBottom()
}

async function mergeLiveList() {
  if (!props.profileId) return
  const probe = await probeXhsConvList(props.profileId)
  if (!probe.cdpOnline) {
    ElMessage.warning(probe.message || 'CDP 离线')
    return
  }
  if (!probe.inIm) {
    ElMessage.warning('请先打开小红书消息页')
    return
  }
  const map: Record<string, number> = {}
  const existing = new Set(
    (await csSessionList(props.profileId))
      .filter((s) => (s.messageCount || 0) > 0)
      .map((s) => s.peerName),
  )
  for (const c of probe.conversations || []) {
    const name = (c.name || '').trim()
    if (!name) continue
    map[name] = c.index
    // 只更新已有对话交互的会话元数据，不把纯列表用户写进会话记录
    if (existing.has(name)) {
      await csSessionTouch({
        profileId: props.profileId,
        peerName: name,
        lastPreview: c.preview || '',
        unreadBadge: c.badge || 0,
      })
    }
  }
  liveIndexByPeer.value = map
  await loadSessions()
}

async function syncSelectedFromBrowser() {
  if (!props.profileId || !selected.value) return
  syncing.value = true
  try {
    const peer = selected.value.peerName
    let idx = liveIndexByPeer.value[peer]
    if (idx == null) {
      const probe = await probeXhsConvList(props.profileId)
      const hit = (probe.conversations || []).find((c) => c.name === peer)
      if (hit) {
        idx = hit.index
        liveIndexByPeer.value = {
          ...liveIndexByPeer.value,
          ...Object.fromEntries(
            (probe.conversations || []).map((c: XhsConvListItem) => [c.name, c.index]),
          ),
        }
      }
    }
    if (idx != null) {
      const clicked = await clickXhsConvItem(props.profileId, idx)
      if (!clicked.ok) {
        ElMessage.warning(clicked.message || '打开会话失败')
      }
    }
    const state = await probeXhsImState(props.profileId)
    const name = state.peerName || state.activeConvName || peer
    const msgs = (state.messages || [])
      .map((m) => ({ direction: m.direction || 'unknown', text: (m.text || '').trim() }))
      .filter((m) => m.text)
    if (!msgs.length) {
      ElMessage.info('当前对话框暂无消息')
      return
    }
    await csMessagesSync({
      profileId: props.profileId,
      peerName: name,
      messages: msgs,
      unreadBadge: 0,
    })
    await loadSessions()
    const sid = sessions.value.find((s) => s.peerName === name)?.id
    if (sid) selectedId.value = sid
    await loadMessages()
    ElMessage.success(`已同步 ${msgs.length} 条`)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    syncing.value = false
  }
}

async function refreshAll() {
  loading.value = true
  try {
    await loadSessions()
    try {
      await mergeLiveList()
    } catch {
      /* CDP 不可用时仍展示本地记录 */
    }
    await loadMessages()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
  }
}

async function onSelect(row: CsSession) {
  selectedId.value = row.id
  await loadMessages()
}

watch(
  () => props.modelValue,
  (open) => {
    if (open) void refreshAll()
  },
)

watch(
  () => props.profileId,
  () => {
    if (props.modelValue) void refreshAll()
  },
)
</script>

<template>
  <el-dialog
    v-model="visible"
    title="会话记录"
    width="920px"
    top="6vh"
    class="cs-chat-dialog"
    destroy-on-close
  >
    <div class="cs-chat" v-loading="loading || syncing">
      <aside class="cs-chat__side">
        <div class="cs-chat__side-head">
          <strong>会话用户</strong>
          <span class="cs-chat__count">{{ sessions.length }}</span>
        </div>
        <div class="cs-chat__list">
          <button
            v-for="s in sessions"
            :key="s.id"
            type="button"
            class="cs-chat__user"
            :class="{ 'is-on': s.id === selectedId }"
            @click="onSelect(s)"
          >
            <div class="cs-chat__user-top">
              <span class="cs-chat__name">{{ s.peerName }}</span>
              <el-badge v-if="s.unreadBadge > 0" :value="s.unreadBadge" :max="99" />
            </div>
            <div class="cs-chat__user-meta">
              <span>{{ s.messageCount }} 条消息</span>
              <span>{{ fmtTime(s.updatedAt) }}</span>
            </div>
            <p class="cs-chat__preview">{{ s.lastPreview || '暂无预览' }}</p>
          </button>
          <div v-if="!sessions.length" class="cs-chat__empty">暂无会话，自动回复或同步浏览器后会出现</div>
        </div>
      </aside>

      <section class="cs-chat__main">
        <div class="cs-chat__main-head">
          <div>
            <strong>{{ selected?.peerName || '未选择会话' }}</strong>
            <span v-if="selected" class="cs-chat__sub">
              共 {{ selected.messageCount }} 条 · 更新 {{ fmtTime(selected.updatedAt) }}
            </span>
          </div>
          <div class="cs-chat__actions">
            <el-button size="small" @click="refreshAll">刷新列表</el-button>
            <el-button
              size="small"
              type="primary"
              plain
              :disabled="!selected"
              :loading="syncing"
              @click="syncSelectedFromBrowser"
            >
              从浏览器同步
            </el-button>
          </div>
        </div>
        <div ref="chatScrollEl" class="cs-chat__msgs">
          <template v-if="selected">
            <div
              v-for="m in messages"
              :key="m.id"
              class="cs-chat__bubble"
              :class="m.direction === 'sent' ? 'is-sent' : 'is-recv'"
            >
              <div class="cs-chat__bubble-meta">
                <span>{{ m.direction === 'sent' ? '我' : selected.peerName }}</span>
                <span>{{ fmtTime(m.createdAt) }}</span>
              </div>
              <div class="cs-chat__bubble-text">{{ m.text }}</div>
            </div>
            <div v-if="!messages.length" class="cs-chat__empty">
              暂无对话记录，可点「从浏览器同步」拉取当前会话
            </div>
          </template>
          <div v-else class="cs-chat__empty">请选择左侧用户</div>
        </div>
      </section>
    </div>
  </el-dialog>
</template>

<style scoped lang="scss">
.cs-chat {
  display: grid;
  grid-template-columns: 280px 1fr;
  height: min(68vh, 620px);
  border: 1px solid var(--border, #eef0f3);
  border-radius: 10px;
  overflow: hidden;
  background: #fff;
}
.cs-chat__side {
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border, #eef0f3);
  background: #fafbfc;
  min-height: 0;
}
.cs-chat__side-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 14px;
  border-bottom: 1px solid var(--border, #eef0f3);
  strong {
    font-size: 13px;
  }
}
.cs-chat__count {
  font-size: 12px;
  color: var(--muted, #888);
}
.cs-chat__list {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 6px;
}
.cs-chat__user {
  width: 100%;
  text-align: left;
  border: none;
  background: transparent;
  border-radius: 8px;
  padding: 10px 10px 8px;
  cursor: pointer;
  color: inherit;
  font: inherit;
  margin-bottom: 2px;
  &:hover {
    background: var(--el-color-primary-light-9, #ecf5ff);
  }
  &.is-on {
    background: var(--el-color-primary-light-8, #d9ecff);
  }
}
.cs-chat__user-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.cs-chat__name {
  font-weight: 600;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.cs-chat__user-meta {
  display: flex;
  justify-content: space-between;
  margin-top: 4px;
  font-size: 11px;
  color: var(--muted, #888);
}
.cs-chat__preview {
  margin: 4px 0 0;
  font-size: 12px;
  color: #666;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.cs-chat__main {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
}
.cs-chat__main-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
  border-bottom: 1px solid var(--border, #eef0f3);
  strong {
    display: block;
    font-size: 14px;
  }
}
.cs-chat__sub {
  display: block;
  margin-top: 4px;
  font-size: 12px;
  color: var(--muted, #888);
}
.cs-chat__actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}
.cs-chat__msgs {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 14px 16px;
  background: #f7f8fa;
}
.cs-chat__bubble {
  max-width: 78%;
  margin-bottom: 12px;
  &.is-recv {
    margin-right: auto;
  }
  &.is-sent {
    margin-left: auto;
    .cs-chat__bubble-text {
      background: var(--el-color-primary, #409eff);
      color: #fff;
    }
    .cs-chat__bubble-meta {
      justify-content: flex-end;
    }
  }
}
.cs-chat__bubble-meta {
  display: flex;
  gap: 8px;
  font-size: 11px;
  color: var(--muted, #888);
  margin-bottom: 4px;
}
.cs-chat__bubble-text {
  display: inline-block;
  padding: 8px 12px;
  border-radius: 10px;
  background: #fff;
  border: 1px solid var(--border, #eaecef);
  font-size: 13px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}
.cs-chat__empty {
  padding: 40px 16px;
  text-align: center;
  color: var(--muted, #888);
  font-size: 13px;
}
</style>
