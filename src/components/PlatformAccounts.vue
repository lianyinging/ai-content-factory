<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { Platform } from '@/types/item'
import {
  authList,
  authLoginCookie,
  authLogout,
  authStatus,
  catbusRun,
  qrImage,
  qrStart,
  qrStatus,
} from '@/api/catbus'
import {
  browserCdpStatusAll,
  deleteBrowserProfile,
  detectBrowsers,
  getBrowserConfig,
  launchBrowserCdp,
  listBrowserProfiles,
  loginBrowserPlatform,
  openBrowserUrl,
  parseProfileSession,
  refreshProfileSession,
  saveBrowserConfig,
  stopBrowserCdp,
  upsertBrowserProfile,
  type BrowserCandidate,
  type BrowserConfig,
  type BrowserProfile,
  type BrowserStatus,
  type PlatformSession,
} from '@/api/cdp'

const props = defineProps<{ platform: Platform; label: string }>()

interface AccountRow {
  account: string
  nickname: string
  loggedIn: boolean
  externalUserId: string
  avatarUrl: string
  updatedAt: string
}

const loading = ref(false)
const accounts = ref<AccountRow[]>([])
const accountTab = ref<'pc' | 'cdp'>('pc')
const drawer = ref(false)
const method = ref<'qr' | 'cookie'>('qr')
const accountName = ref('default')
const cookieText = ref('')
const busy = ref(false)
const checkBusy = ref('')
const qrDataUrl = ref('')
const qrHint = ref('')
let qrTimer: ReturnType<typeof setInterval> | null = null

const cdpBusy = ref(false)
const cdpConfig = ref<BrowserConfig | null>(null)
const cdpProfiles = ref<BrowserProfile[]>([])
const cdpStatuses = ref<BrowserStatus[]>([])
const cdpCandidates = ref<BrowserCandidate[]>([])
const newProfileName = ref('')

function stopQrPoll() {
  if (qrTimer) {
    clearInterval(qrTimer)
    qrTimer = null
  }
}

function parseAccounts(env: Record<string, unknown>): AccountRow[] {
  const data = env.data
  const list: unknown[] = Array.isArray(data)
    ? data
    : data && typeof data === 'object' && Array.isArray((data as { accounts?: unknown[] }).accounts)
      ? (data as { accounts: unknown[] }).accounts
      : data && typeof data === 'object' && Array.isArray((data as { list?: unknown[] }).list)
        ? (data as { list: unknown[] }).list
        : []

  return list
    .map((raw) => {
      if (!raw || typeof raw !== 'object') return null
      const r = raw as Record<string, unknown>
      const user =
        r.user && typeof r.user === 'object' ? (r.user as Record<string, unknown>) : {}
      const account = String(r.account || r.name || 'default')
      return {
        account,
        nickname: String(user.nickname || user.name || r.nickname || account || '未命名账号'),
        loggedIn: r.logged_in === true || r.loggedIn === true || Boolean(user.id || user.uid),
        externalUserId: String(user.id || user.uid || ''),
        avatarUrl: String(user.avatar || user.avatarUrl || ''),
        updatedAt: String(r.updated_at || r.updatedAt || ''),
      } satisfies AccountRow
    })
    .filter(Boolean) as AccountRow[]
}

/** auth list/status 不含头像；用 user get 补全（带会话缓存，避免反复请求） */
const avatarCache = new Map<string, string>()

function avatarCacheKey(account: string, userId: string) {
  return `${props.platform}:${account}:${userId}`
}

async function enrichAccountAvatars(rows: AccountRow[]): Promise<AccountRow[]> {
  return Promise.all(
    rows.map(async (row) => {
      if (row.avatarUrl || !row.externalUserId) return row
      const key = avatarCacheKey(row.account, row.externalUserId)
      const cached = avatarCache.get(key)
      if (cached) return { ...row, avatarUrl: cached }
      try {
        const env = await catbusRun(
          [props.platform, 'user', 'get', row.externalUserId, '-a', row.account || 'default'],
          { timeoutSecs: 60 },
        )
        if (env.ok === false || env.error) return row
        const data = (env.data && typeof env.data === 'object' ? env.data : {}) as Record<
          string,
          unknown
        >
        const user =
          (data.user && typeof data.user === 'object'
            ? (data.user as Record<string, unknown>)
            : data) || {}
        const avatar = String(user.avatar || user.avatarUrl || data.avatar || '')
        if (!avatar) return row
        avatarCache.set(key, avatar)
        return {
          ...row,
          avatarUrl: avatar,
          nickname: row.nickname || String(user.nickname || user.name || row.nickname),
        }
      } catch {
        return row
      }
    }),
  )
}

async function refresh() {
  loading.value = true
  try {
    const env = await authList(props.platform)
    if (!env.ok) {
      ElMessage.error(env.error?.message || env.error?.hint || '获取账号失败')
      accounts.value = []
      return
    }
    let rows = parseAccounts(env as Record<string, unknown>)
    if (!rows.length) {
      const st = await authStatus(props.platform, 'default')
      const data = (st.data && typeof st.data === 'object' ? st.data : {}) as Record<
        string,
        unknown
      >
      const user =
        data.user && typeof data.user === 'object'
          ? (data.user as Record<string, unknown>)
          : {}
      const loggedIn = Boolean(data.logged_in || data.loggedIn || user.id)
      if (loggedIn) {
        rows = [
          {
            account: 'default',
            nickname: String(user.nickname || user.name || 'default'),
            loggedIn,
            externalUserId: String(user.id || user.uid || ''),
            avatarUrl: String(user.avatar || user.avatarUrl || ''),
            updatedAt: '',
          },
        ]
      }
    }
    accounts.value = await enrichAccountAvatars(rows)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
  }
}

async function refreshQrImage() {
  const img = await qrImage(props.platform)
  if (img.ok && img.base64) {
    qrDataUrl.value = `data:${img.mime || 'image/png'};base64,${img.base64}`
    qrHint.value = '请使用 App 扫码登录'
  }
}

async function startQr() {
  busy.value = true
  qrDataUrl.value = ''
  qrHint.value = '正在生成二维码…'
  stopQrPoll()
  try {
    await qrStart(props.platform, accountName.value.trim() || 'default')
    let tries = 0
    qrTimer = setInterval(async () => {
      tries += 1
      try {
        await refreshQrImage()
        const st = await qrStatus(props.platform)
        if (st.status === 'success') {
          stopQrPoll()
          ElMessage.success('搜索账号绑定成功')
          drawer.value = false
          await refresh()
        } else if (st.status === 'failed') {
          stopQrPoll()
          ElMessage.error(st.error || '扫码登录失败')
          qrHint.value = st.error || '登录失败'
        }
      } catch {
        /* ignore */
      }
      if (tries > 120) {
        stopQrPoll()
        qrHint.value = '超时，请重试'
      }
    }, 2000)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busy.value = false
  }
}

async function submitCookie() {
  const cookie = cookieText.value.trim()
  if (!cookie) {
    ElMessage.warning('请粘贴 Cookie')
    return
  }
  busy.value = true
  try {
    const env = await authLoginCookie(
      props.platform,
      cookie,
      accountName.value.trim() || 'default',
    )
    if (!env.ok) {
      ElMessage.error(env.error?.message || env.error?.hint || 'Cookie 登录失败')
      return
    }
    ElMessage.success('已绑定搜索账号')
    drawer.value = false
    cookieText.value = ''
    await refresh()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busy.value = false
  }
}

async function onCheck(account: string) {
  checkBusy.value = account
  try {
    const st = await authStatus(props.platform, account)
    if (st.ok) ElMessage.success('账号状态正常')
    else ElMessage.warning(st.error?.message || st.error?.hint || '账号可能已失效')
    await refresh()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    checkBusy.value = ''
  }
}

async function onDelete(account: string) {
  try {
    await ElMessageBox.confirm(`删除搜索账号「${account}」？`, '确认', { type: 'warning' })
  } catch {
    return
  }
  busy.value = true
  try {
    const env = await authLogout(props.platform, account)
    if (!env.ok) {
      ElMessage.error(env.error?.message || '删除失败')
      return
    }
    ElMessage.success('已删除')
    await refresh()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    busy.value = false
  }
}

function openLogin() {
  drawer.value = true
  method.value = 'qr'
  accountName.value = 'default'
  qrDataUrl.value = ''
  qrHint.value = ''
}

function idLabel(a: AccountRow) {
  if (props.platform === 'xhs' && a.externalUserId) return `${props.label} ${a.externalUserId}`
  if (a.externalUserId) return `UID ${a.externalUserId}`
  return a.account
}

async function refreshCdp() {
  cdpBusy.value = true
  try {
    cdpCandidates.value = await detectBrowsers()
    cdpConfig.value = await getBrowserConfig()
    cdpProfiles.value = await listBrowserProfiles()
    const sum = await browserCdpStatusAll()
    cdpStatuses.value = sum.profiles || []
    if (cdpConfig.value && !cdpConfig.value.browserExecutable && cdpCandidates.value[0]) {
      cdpConfig.value.browserExecutable = cdpCandidates.value[0].path
      cdpConfig.value.browserKind = cdpCandidates.value[0].kind
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    cdpBusy.value = false
  }
}

async function saveCdpConfig() {
  if (!cdpConfig.value) return
  cdpBusy.value = true
  try {
    cdpConfig.value = await saveBrowserConfig(cdpConfig.value)
    ElMessage.success('浏览器配置已保存')
    await refreshCdp()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    cdpBusy.value = false
  }
}

async function addProfile() {
  const name = newProfileName.value.trim() || `Profile ${cdpProfiles.value.length + 1}`
  try {
    await upsertBrowserProfile({ name, platforms: props.platform === 'xhs' ? 'xhs' : 'douyin' })
    newProfileName.value = ''
    await refreshCdp()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

function statusOf(id: string) {
  return cdpStatuses.value.find((s) => s.profileId === id)
}

async function launchProfile(id: string) {
  cdpBusy.value = true
  try {
    await launchBrowserCdp(id)
    ElMessage.success('已启动')
    await refreshCdp()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    cdpBusy.value = false
  }
}

async function stopProfile(id: string) {
  cdpBusy.value = true
  try {
    await stopBrowserCdp(id)
    await refreshCdp()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    cdpBusy.value = false
  }
}

async function loginCreator(id: string) {
  cdpBusy.value = true
  try {
    await loginBrowserPlatform(props.platform, id)
    ElMessage.success('已打开创作者登录页，请在自动化浏览器中完成登录')
    await refreshCdp()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    cdpBusy.value = false
  }
}

async function openCreator(id: string) {
  const url =
    props.platform === 'xhs'
      ? 'https://creator.xiaohongshu.com/publish/publish?source=official'
      : 'https://creator.douyin.com/'
  try {
    await openBrowserUrl(url, id)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

function sessionOf(p: BrowserProfile): PlatformSession | undefined {
  const s = parseProfileSession(p.sessionJson)
  return props.platform === 'xhs' ? s.xhs : s.douyin
}

/** 当前平台已探测为已登录的 CDP Profile 数量 */
const cdpLoggedInCount = computed(
  () => cdpProfiles.value.filter((p) => Boolean(sessionOf(p)?.loggedIn)).length,
)

function sessionLabel(p: BrowserProfile) {
  const s = sessionOf(p)
  if (!s) return '未探测会话'
  if (s.loggedIn && s.nickname) return `已登录 · ${s.nickname}`
  if (s.loggedIn) return '已登录'
  return '未登录 / 未知'
}

async function onRefreshSession(p: BrowserProfile) {
  const st = statusOf(p.id)
  if (!st?.cdpOnline) {
    ElMessage.warning('请先启动该 Profile')
    return
  }
  cdpBusy.value = true
  try {
    const { profile } = await refreshProfileSession(p, [props.platform === 'xhs' ? 'xhs' : 'douyin'])
    const idx = cdpProfiles.value.findIndex((x) => x.id === p.id)
    if (idx >= 0) cdpProfiles.value[idx] = profile
    ElMessage.success(sessionLabel(profile))
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    cdpBusy.value = false
  }
}

async function onDeleteProfile(p: BrowserProfile) {
  if (p.id === 'default') {
    ElMessage.warning('默认 Profile 不可删除')
    return
  }
  try {
    await ElMessageBox.confirm(`确定删除 Profile「${p.name}」？`, '删除确认', { type: 'warning' })
    await deleteBrowserProfile(p.id)
    ElMessage.success('已删除')
    await refreshCdp()
  } catch (e) {
    if (e !== 'cancel') ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

onMounted(async () => {
  await refresh()
  await refreshCdp()
})
onUnmounted(stopQrPoll)
</script>

<template>
  <div v-loading="loading">
    <div class="mf-ops__role-callout" role="note">
      <div class="mf-ops__role-callout-item">
        <strong>搜索账号 · 只搜不发</strong>
        <p>
          绑定专门用来<strong>搜内容、抓数据</strong>的账号（扫码 / Cookie 均可）。不要用要发帖的主号去做搜索，降低风控关联。
        </p>
      </div>
      <div class="mf-ops__role-callout-item mf-ops__role-callout-item--cdp">
        <strong>发布 · 桌面内嵌 CDP</strong>
        <p>
          在下方「桌面 CDP」启动独立 Chrome/Edge Profile，登录创作者中心后，可在发布中心拟人化填表/发布。
        </p>
      </div>
    </div>

    <section class="mf-ops__panel">
      <div class="mf-ops__panel-head">
        <div class="mf-ops__seg">
          <button
            type="button"
            class="mf-ops__seg-btn"
            :class="{ 'is-active': accountTab === 'pc' }"
            @click="accountTab = 'pc'"
          >
            搜索账号 · {{ accounts.length }}
          </button>
          <button
            type="button"
            class="mf-ops__seg-btn"
            :class="{ 'is-active': accountTab === 'cdp' }"
            @click="accountTab = 'cdp'; refreshCdp()"
          >
            桌面 CDP · {{ cdpLoggedInCount }}
          </button>
        </div>
        <div v-if="accountTab === 'pc'" class="mf-ops__panel-head-actions">
          <el-button @click="refresh" :loading="loading">刷新</el-button>
          <el-button type="primary" @click="openLogin">绑定搜索账号</el-button>
        </div>
        <div v-else class="mf-ops__panel-head-actions">
          <el-button size="small" :loading="cdpBusy" @click="refreshCdp">刷新状态</el-button>
        </div>
      </div>

      <div class="mf-ops__panel-body">
        <template v-if="accountTab === 'pc'">
          <div v-if="!accounts.length" class="mf-ops__empty">
            <strong>还没有绑定搜索账号</strong>
            <p>请绑定专门用来搜索 / 抓取的账号。发帖请走创作者中心，不要用主号去做搜索。</p>
            <el-button type="primary" @click="openLogin">绑定搜索账号</el-button>
          </div>
          <div v-else class="mf-ops__grid">
            <article v-for="a in accounts" :key="a.account" class="mf-ops__card">
              <div class="mf-ops__card-head">
                <div class="mf-ops__card-avatar">
                  <img
                    v-if="a.avatarUrl"
                    :src="a.avatarUrl"
                    alt=""
                    referrerpolicy="no-referrer"
                  />
                  <span v-else>{{ (a.nickname || '?').slice(0, 1) }}</span>
                </div>
                <div class="mf-ops__card-meta">
                  <strong>{{ a.nickname || '未命名账号' }}</strong>
                  <em>{{ idLabel(a) }}</em>
                </div>
                <el-tag size="small" :type="a.loggedIn ? 'success' : 'info'">
                  {{ a.loggedIn ? '正常' : '未知' }}
                </el-tag>
              </div>
              <div class="mf-ops__card-stats">
                <div><span>类型</span><b>PC 搜索</b></div>
                <div><span>Cookie</span><b>{{ a.loggedIn ? '已绑定' : '无' }}</b></div>
                <div><span>账号</span><b>{{ a.account }}</b></div>
              </div>
              <div class="mf-ops__card-foot">
                <span class="mf-ops__mono">{{ a.externalUserId || a.account }}</span>
                <span>
                  <el-button size="small" :loading="checkBusy === a.account" @click="onCheck(a.account)">
                    检查
                  </el-button>
                  <el-button size="small" type="danger" plain @click="onDelete(a.account)">
                    删除
                  </el-button>
                </span>
              </div>
            </article>
          </div>
        </template>

        <div v-else class="cdp-panel" v-loading="cdpBusy">
          <div v-if="cdpConfig" class="cdp-config">
            <el-form label-width="108px" size="small">
              <el-form-item label="浏览器">
                <el-select
                  v-model="cdpConfig.browserExecutable"
                  filterable
                  allow-create
                  style="width: 100%"
                >
                  <el-option
                    v-for="c in cdpCandidates"
                    :key="c.path"
                    :label="`${c.label} — ${c.path}`"
                    :value="c.path"
                  />
                </el-select>
              </el-form-item>
              <el-form-item>
                <el-button type="primary" size="small" @click="saveCdpConfig">保存配置</el-button>
              </el-form-item>
            </el-form>
          </div>

          <div class="cdp-add">
            <el-input
              v-model="newProfileName"
              size="small"
              placeholder="新 Profile 名称"
              style="width: 200px"
            />
            <el-button size="small" type="primary" plain @click="addProfile">添加 Profile</el-button>
          </div>

          <div class="mf-ops__grid">
            <article v-for="p in cdpProfiles" :key="p.id" class="mf-ops__card">
              <div class="mf-ops__card-head">
                <div class="mf-ops__card-avatar">
                  <img
                    v-if="sessionOf(p)?.avatarUrl"
                    :src="sessionOf(p)?.avatarUrl"
                    alt=""
                    referrerpolicy="no-referrer"
                  />
                  <span v-else>{{ (sessionOf(p)?.nickname || p.name || '?').slice(0, 1) }}</span>
                </div>
                <div class="mf-ops__card-meta">
                  <strong>{{ sessionOf(p)?.nickname || p.name }}</strong>
                  <em>
                    {{ sessionLabel(p) }} · 端口 {{ p.debugPort }}
                    <template v-if="sessionOf(p)?.redId"> · 号 {{ sessionOf(p)?.redId }}</template>
                  </em>
                </div>
                <el-tag
                  size="small"
                  :type="statusOf(p.id)?.cdpOnline ? 'success' : 'info'"
                >
                  {{ statusOf(p.id)?.cdpOnline ? 'CDP 在线' : '离线' }}
                </el-tag>
              </div>
              <div class="mf-ops__card-stats">
                <div><span>粉丝</span><b>{{ sessionOf(p)?.followers || '—' }}</b></div>
                <div><span>关注</span><b>{{ sessionOf(p)?.following || '—' }}</b></div>
                <div><span>获赞</span><b>{{ sessionOf(p)?.likesCollect || '—' }}</b></div>
              </div>
              <div class="mf-ops__card-foot cdp-card-foot">
                <div class="cdp-card-foot__meta">
                  <span class="mf-ops__mono">{{ p.id }}</span>
                  <span v-if="p.name && p.name !== sessionOf(p)?.nickname" class="cdp-card-foot__name">
                    {{ p.name }}
                  </span>
                </div>
                <div class="cdp-card-foot__actions">
                  <el-button size="small" @click="launchProfile(p.id)">启动</el-button>
                  <el-button size="small" @click="stopProfile(p.id)">停止</el-button>
                  <el-button size="small" type="primary" plain @click="loginCreator(p.id)">
                    登录
                  </el-button>
                  <el-button size="small" @click="onRefreshSession(p)">刷新会话</el-button>
                  <el-button size="small" @click="openCreator(p.id)">创作者中心</el-button>
                  <el-button
                    v-if="p.id !== 'default'"
                    size="small"
                    type="danger"
                    plain
                    @click="onDeleteProfile(p)"
                  >
                    删除
                  </el-button>
                </div>
              </div>
            </article>
          </div>
        </div>
      </div>
    </section>

    <el-drawer v-model="drawer" title="绑定搜索账号" size="420px" @close="stopQrPoll">
      <el-form label-position="top">
        <el-form-item label="账号标识">
          <el-input v-model="accountName" placeholder="default" />
        </el-form-item>
        <el-form-item label="方式">
          <el-radio-group v-model="method">
            <el-radio-button value="qr">扫码</el-radio-button>
            <el-radio-button value="cookie">Cookie</el-radio-button>
          </el-radio-group>
        </el-form-item>

        <template v-if="method === 'qr'">
          <div class="qr-box">
            <img v-if="qrDataUrl" :src="qrDataUrl" alt="qrcode" />
            <div v-else class="qr-placeholder">{{ qrHint || '点击下方生成二维码' }}</div>
          </div>
          <p class="hint">{{ qrHint }}</p>
          <el-button type="primary" :loading="busy" style="width: 100%" @click="startQr">
            生成二维码
          </el-button>
        </template>

        <template v-else>
          <el-form-item label="Cookie">
            <el-input
              v-model="cookieText"
              type="textarea"
              :rows="8"
              placeholder="粘贴浏览器 Cookie 字符串"
            />
          </el-form-item>
          <el-button type="primary" :loading="busy" style="width: 100%" @click="submitCookie">
            导入并绑定
          </el-button>
        </template>
      </el-form>
    </el-drawer>
  </div>
</template>

<style scoped lang="scss">
.qr-box {
  width: 240px;
  height: 240px;
  margin: 0 auto 12px;
  border: 1px dashed var(--line);
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  background: #fafafa;

  img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
}

.qr-placeholder {
  padding: 16px;
  text-align: center;
  color: var(--muted);
  font-size: 13px;
}

.hint {
  margin: 0 0 12px;
  color: var(--muted);
  font-size: 13px;
  text-align: center;
}

.cdp-panel {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.cdp-add {
  display: flex;
  align-items: center;
  gap: 8px;
}

.cdp-config {
  max-width: 720px;
}

.path {
  font-size: 11px;
  font-weight: 500;
  word-break: break-all;
}

.cdp-card-foot {
  flex-direction: column;
  align-items: stretch;
  gap: 10px;
}

.cdp-card-foot__meta {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.cdp-card-foot__name {
  font-size: 11px;
  color: var(--color-text-3, #8a919f);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cdp-card-foot__actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;

  /* Element Plus 相邻按钮默认 margin-left，换行后会对不齐 */
  :deep(.el-button + .el-button) {
    margin-left: 0;
  }
}
</style>
