<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { ElMessage } from 'element-plus'
import type { DoctorInfo } from '@/types/item'
import { catbusDoctor, catbusRun } from '@/api/catbus'
import ModelConfig from '@/views/ModelConfig.vue'
import BrowserAutomation from '@/views/BrowserAutomation.vue'

const tab = ref('doctor')
const loading = ref(false)
const doctor = ref<DoctorInfo | null>(null)
const doctorCli = ref('')

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
</style>
