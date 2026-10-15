<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  browserCdpStatusAll,
  browserSmoke,
  deleteBrowserProfile,
  detectBrowsers,
  getBrowserConfig,
  launchBrowserCdp,
  listBrowserProfiles,
  loginBrowserPlatform,
  parseProfileSession,
  refreshProfileSession,
  saveBrowserConfig,
  stopBrowserCdp,
  upsertBrowserProfile,
  type BrowserCandidate,
  type BrowserConfig,
  type BrowserProfile,
  type BrowserStatus,
} from '@/api/cdp'

const busy = ref(false)
const config = ref<BrowserConfig | null>(null)
const candidates = ref<BrowserCandidate[]>([])
const profiles = ref<BrowserProfile[]>([])
const statuses = ref<BrowserStatus[]>([])
const newName = ref('')
const smokeMsg = ref('')

async function refresh() {
  busy.value = true
  try {
    candidates.value = await detectBrowsers()
    config.value = await getBrowserConfig()
    profiles.value = await listBrowserProfiles()
    const sum = await browserCdpStatusAll()
    statuses.value = sum.profiles || []
    if (config.value && !config.value.browserExecutable && candidates.value[0]) {
      config.value.browserExecutable = candidates.value[0].path
      config.value.browserKind = candidates.value[0].kind
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busy.value = false
  }
}

function statusOf(id: string) {
  return statuses.value.find((s) => s.profileId === id)
}

function sessionLabel(p: BrowserProfile) {
  const s = parseProfileSession(p.sessionJson)
  const parts: string[] = []
  if (s.xhs?.nickname) parts.push(`小红书:${s.xhs.nickname}`)
  else if (s.xhs?.loggedIn) parts.push('小红书:已登录')
  if (s.douyin?.nickname) parts.push(`抖音:${s.douyin.nickname}`)
  else if (s.douyin?.loggedIn) parts.push('抖音:已登录')
  return parts.join(' · ') || '未探测会话'
}

async function save() {
  if (!config.value) return
  busy.value = true
  try {
    config.value = await saveBrowserConfig(config.value)
    ElMessage.success('已保存')
    await refresh()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busy.value = false
  }
}

async function addProfile() {
  try {
    await upsertBrowserProfile({
      name: newName.value.trim() || `Profile ${profiles.value.length + 1}`,
      platforms: 'both',
    })
    newName.value = ''
    await refresh()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function launch(id: string) {
  busy.value = true
  try {
    await launchBrowserCdp(id)
    await refresh()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busy.value = false
  }
}

async function stop(id: string) {
  busy.value = true
  try {
    await stopBrowserCdp(id)
    await refresh()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busy.value = false
  }
}

async function login(id: string, platform: 'xhs' | 'douyin') {
  busy.value = true
  try {
    await loginBrowserPlatform(platform, id)
    ElMessage.success('已打开登录页')
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busy.value = false
  }
}

async function probe(p: BrowserProfile) {
  if (!statusOf(p.id)?.cdpOnline) {
    ElMessage.warning('请先启动')
    return
  }
  busy.value = true
  try {
    const { profile } = await refreshProfileSession(p, ['xhs', 'douyin'])
    const i = profiles.value.findIndex((x) => x.id === p.id)
    if (i >= 0) profiles.value[i] = profile
    ElMessage.success(sessionLabel(profile))
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busy.value = false
  }
}

async function smoke(p: BrowserProfile) {
  if (!statusOf(p.id)?.cdpOnline) {
    ElMessage.warning('请先启动')
    return
  }
  busy.value = true
  smokeMsg.value = ''
  try {
    const r = await browserSmoke(p.debugPort)
    smokeMsg.value = String(r.message || 'smoke ok')
    ElMessage.success(smokeMsg.value)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busy.value = false
  }
}

async function remove(p: BrowserProfile) {
  if (p.id === 'default') {
    ElMessage.warning('默认 Profile 不可删除')
    return
  }
  try {
    await ElMessageBox.confirm(`删除「${p.name}」？`, '确认', { type: 'warning' })
    await deleteBrowserProfile(p.id)
    await refresh()
  } catch (e) {
    if (e !== 'cancel') ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

onMounted(refresh)
</script>

<template>
  <div class="ba" v-loading="busy">
    <section class="mf-ops__panel" style="flex: none; max-width: 920px">
      <div class="mf-ops__panel-head">
        <strong>浏览器自动化</strong>
        <el-button size="small" @click="refresh">刷新</el-button>
      </div>
      <div class="mf-ops__panel-body">
        <p class="lede">
          检测本机 Chrome/Edge，管理 CDP Profile，登录创作者中心后可用于发布与自动运营。
        </p>
        <el-form v-if="config" label-width="120px" size="small">
          <el-form-item label="浏览器路径">
            <el-select v-model="config.browserExecutable" filterable allow-create style="width: 100%">
              <el-option
                v-for="c in candidates"
                :key="c.path"
                :label="`${c.label} — ${c.path}`"
                :value="c.path"
              />
            </el-select>
          </el-form-item>
          <el-form-item label="自动启动">
            <el-switch v-model="config.autoLaunch" />
          </el-form-item>
          <el-form-item>
            <el-button type="primary" @click="save">保存配置</el-button>
          </el-form-item>
        </el-form>

        <div class="add">
          <el-input v-model="newName" size="small" placeholder="新 Profile 名称" style="width: 220px" />
          <el-button size="small" type="primary" plain @click="addProfile">添加 Profile</el-button>
        </div>

        <div v-for="p in profiles" :key="p.id" class="row">
          <div>
            <strong>{{ p.name }}</strong>
            <em>:{{ p.debugPort }} · {{ statusOf(p.id)?.cdpOnline ? '在线' : '离线' }}</em>
            <p>{{ sessionLabel(p) }}</p>
          </div>
          <div class="acts">
            <el-button size="small" @click="launch(p.id)">启动</el-button>
            <el-button size="small" @click="stop(p.id)">停止</el-button>
            <el-button size="small" @click="login(p.id, 'xhs')">登录小红书</el-button>
            <el-button size="small" @click="login(p.id, 'douyin')">登录抖音</el-button>
            <el-button size="small" type="primary" plain @click="probe(p)">探测会话</el-button>
            <el-button size="small" @click="smoke(p)">Smoke</el-button>
            <el-button
              v-if="p.id !== 'default'"
              size="small"
              type="danger"
              plain
              @click="remove(p)"
            >
              删除
            </el-button>
          </div>
        </div>
        <p v-if="smokeMsg" class="smoke">{{ smokeMsg }}</p>
      </div>
    </section>
  </div>
</template>

<style scoped>
.lede {
  margin: 0 0 14px;
  color: var(--muted);
  font-size: 13px;
}
.add {
  display: flex;
  gap: 8px;
  margin: 12px 0 16px;
}
.row {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 0;
  border-top: 1px solid var(--border, #eee);
}
.row em {
  margin-left: 8px;
  color: var(--muted);
  font-style: normal;
  font-size: 12px;
}
.row p {
  margin: 4px 0 0;
  font-size: 12px;
  color: var(--muted);
}
.acts {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  justify-content: flex-end;
  max-width: 520px;
}
.smoke {
  margin-top: 12px;
  font-size: 13px;
}
</style>
