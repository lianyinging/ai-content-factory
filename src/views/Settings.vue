<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { openUrl } from '@tauri-apps/plugin-opener'
import type { DoctorInfo } from '@/types/item'
import { catbusDoctor, catbusRun } from '@/api/catbus'
import {
  checkAppUpdate,
  formatOsLabel,
  getSystemInfo,
  RELEASES_PAGE,
  type SystemInfo,
  type UpdateCheckResult,
} from '@/api/system'
import ModelConfig from '@/views/ModelConfig.vue'
import BrowserAutomation from '@/views/BrowserAutomation.vue'

const tab = ref('doctor')
const loading = ref(false)
const doctor = ref<DoctorInfo | null>(null)
const doctorCli = ref('')

const sysLoading = ref(false)
const system = ref<SystemInfo | null>(null)
const updateLoading = ref(false)
const updateInfo = ref<UpdateCheckResult | null>(null)

async function refresh() {
  loading.value = true
  try {
    doctor.value = await catbusDoctor()
    if (doctor.value.ready) {
      const env = await catbusRun(['doctor'], { timeoutSecs: 60 })
      doctorCli.value = env.ok
        ? 'catbus doctor 通过'
        : env.error?.message || env.error?.hint || 'catbus doctor 失败'
    } else {
      doctorCli.value = ''
    }
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
  }
}

async function refreshSystem() {
  sysLoading.value = true
  try {
    system.value = await getSystemInfo()
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    sysLoading.value = false
  }
}

async function onCheckUpdate() {
  updateLoading.value = true
  try {
    if (!system.value) await refreshSystem()
    const r = await checkAppUpdate()
    updateInfo.value = r
    if (r.hasUpdate) ElMessage.success(r.message)
    else ElMessage.info(r.message)
  } catch (e) {
    updateInfo.value = null
    ElMessage.error(e instanceof Error ? e.message : String(e))
  } finally {
    updateLoading.value = false
  }
}

async function openDownload() {
  const url = updateInfo.value?.downloadUrl || updateInfo.value?.pageUrl || RELEASES_PAGE
  try {
    await openUrl(url)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

async function openReleasesPage() {
  try {
    await openUrl(RELEASES_PAGE)
  } catch (e) {
    ElMessage.error(e instanceof Error ? e.message : String(e))
  }
}

watch(tab, (name) => {
  if (name === 'about' && !system.value) void refreshSystem()
})

onMounted(refresh)
</script>

<template>
  <div class="settings">
    <el-tabs v-model="tab">
      <el-tab-pane label="环境诊断" name="doctor">
        <section class="mf-ops__panel" style="flex: none; max-width: 820px">
          <div class="mf-ops__panel-head">
            <strong>环境诊断</strong>
            <div class="mf-ops__panel-head-actions">
              <el-button type="primary" :loading="loading" @click="refresh">重新检测</el-button>
            </div>
          </div>
          <div class="mf-ops__panel-body">
            <p class="lede">检查内置 Node、catbus-cli 与本地 CATBUS_HOME。</p>
            <el-descriptions v-if="doctor" :column="1" border>
              <el-descriptions-item label="状态">
                <el-tag :type="doctor.ready ? 'success' : 'danger'">
                  {{ doctor.ready ? '就绪' : '未就绪' }}
                </el-tag>
              </el-descriptions-item>
              <el-descriptions-item label="Node">{{ doctor.node || '—' }}</el-descriptions-item>
              <el-descriptions-item label="catbus-cli">{{ doctor.cli || '—' }}</el-descriptions-item>
              <el-descriptions-item label="CATBUS_HOME">
                {{ doctor.catbusHome || '—' }}
              </el-descriptions-item>
              <el-descriptions-item label="插件目录">{{ doctor.pluginDir || '—' }}</el-descriptions-item>
              <el-descriptions-item label="错误">{{ doctor.error || '—' }}</el-descriptions-item>
              <el-descriptions-item label="CLI doctor">{{ doctorCli || '—' }}</el-descriptions-item>
            </el-descriptions>
          </div>
        </section>
      </el-tab-pane>
      <el-tab-pane label="大模型配置" name="model">
        <ModelConfig />
      </el-tab-pane>
      <el-tab-pane label="浏览器自动化" name="browser">
        <BrowserAutomation />
      </el-tab-pane>
      <el-tab-pane label="系统信息" name="about">
        <section class="mf-ops__panel" style="flex: none; max-width: 820px">
          <div class="mf-ops__panel-head">
            <strong>系统信息</strong>
            <div class="mf-ops__panel-head-actions">
              <el-button :loading="sysLoading" @click="refreshSystem">刷新</el-button>
              <el-button type="primary" :loading="updateLoading" @click="onCheckUpdate">
                检查更新
              </el-button>
            </div>
          </div>
          <div class="mf-ops__panel-body" v-loading="sysLoading && !system">
            <p class="lede">应用版本、运行系统与本机数据目录。</p>
            <el-descriptions v-if="system" :column="1" border>
              <el-descriptions-item label="应用名称">
                {{ system.appName || '—' }}
              </el-descriptions-item>
              <el-descriptions-item label="版本号">
                {{ system.version || '—' }}
              </el-descriptions-item>
              <el-descriptions-item label="应用标识">
                {{ system.identifier || '—' }}
              </el-descriptions-item>
              <el-descriptions-item label="操作系统">
                {{ formatOsLabel(system.os, system.arch) }}
              </el-descriptions-item>
              <el-descriptions-item label="Tauri">
                {{ system.tauriVersion || '—' }}
              </el-descriptions-item>
              <el-descriptions-item label="数据目录">
                {{ system.appDataDir || '—' }}
              </el-descriptions-item>
              <el-descriptions-item label="资源目录">
                {{ system.resourceDir || '—' }}
              </el-descriptions-item>
            </el-descriptions>

            <div v-if="updateInfo" class="update-box" :class="{ 'is-new': updateInfo.hasUpdate }">
              <div class="update-box__head">
                <strong>{{ updateInfo.message }}</strong>
                <el-tag size="small" :type="updateInfo.hasUpdate ? 'warning' : 'success'">
                  {{ updateInfo.hasUpdate ? '有新版本' : '已最新' }}
                </el-tag>
              </div>
              <p v-if="updateInfo.hasUpdate" class="update-box__meta">
                最新 {{ updateInfo.latestVersion }}
                <template v-if="updateInfo.publishedAt"> · {{ updateInfo.publishedAt }}</template>
              </p>
              <p v-if="updateInfo.notes" class="update-box__notes">{{ updateInfo.notes }}</p>
              <div class="update-box__actions">
                <el-button
                  v-if="updateInfo.hasUpdate"
                  type="primary"
                  size="small"
                  @click="openDownload"
                >
                  下载安装包
                </el-button>
                <el-button size="small" @click="openReleasesPage">打开发布目录</el-button>
              </div>
            </div>
          </div>
        </section>
      </el-tab-pane>
    </el-tabs>
  </div>
</template>

<style scoped>
.settings {
  height: 100%;
  overflow: auto;
}
.lede {
  margin: 0 0 14px;
  color: var(--muted);
  font-size: 13px;
}
.update-box {
  margin-top: 16px;
  padding: 12px 14px;
  border-radius: 10px;
  border: 1px solid var(--border, #e5e7eb);
  background: #f8fafc;
}
.update-box.is-new {
  border-color: color-mix(in srgb, var(--el-color-warning) 40%, transparent);
  background: color-mix(in srgb, var(--el-color-warning) 8%, #fff);
}
.update-box__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.update-box__meta,
.update-box__notes {
  margin: 8px 0 0;
  font-size: 13px;
  color: var(--muted, #666);
  line-height: 1.5;
  white-space: pre-wrap;
}
.update-box__actions {
  margin-top: 12px;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
</style>
