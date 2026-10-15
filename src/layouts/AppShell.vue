<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  DArrowLeft,
  DArrowRight,
  Document,
  Setting,
  Cpu,
  Picture,
  DataBoard,
} from '@element-plus/icons-vue'
import type { MenuInstance } from 'element-plus'
import type { Platform } from '@/types/item'
import brandLogo from '@/assets/logo.png'
import { useLibraryAnalyzeStore } from '@/state/libraryAnalyze'
import { useCompetitorMonitorStore } from '@/state/competitorMonitor'
import { useIndustryInsightStore } from '@/state/industryInsight'

const { hasActive: analyzeBusy, activeTitle: analyzeBusyTitle } = useLibraryAnalyzeStore()
const { resumeIfNeeded: resumeCompetitorMonitor } = useCompetitorMonitorStore()
const { resumeIfNeeded: resumeIndustryInsight } = useIndustryInsightStore()

const COLLAPSE_KEY = 'ai-factory.sidebarCollapsed'

const route = useRoute()
const router = useRouter()
const collapsed = ref(false)
const sideMenuRef = ref<MenuInstance>()
/** 展开态下「自媒体工厂」是否打开；折叠时先收起，避免展开瞬间高度跳动 */
const mediaOpen = ref(true)
let openMediaTimer: ReturnType<typeof setTimeout> | null = null
/** 折叠/展开时程序化开关子菜单，不记作用户手动关闭 */
let ignoreMediaToggle = false

const platform = computed(() => (route.meta.platform as Platform | undefined) || null)
/** 仪表盘独立全宽，不展示「小红书运营」二级侧栏 */
const isDashboard = computed(() => route.path.startsWith('/xhs/dashboard'))

const primaryActive = computed(() => {
  if (isDashboard.value) return 'dashboard'
  if (platform.value === 'xhs') return 'media-xhs'
  if (platform.value === 'douyin') return 'media-douyin'
  if (route.path.startsWith('/knowledge')) return 'knowledge'
  if (route.path.startsWith('/assets')) return 'assets'
  if (route.path.startsWith('/settings')) return 'settings'
  return ''
})

const moduleTitle = computed(() => {
  if (platform.value === 'xhs') return { title: '小红书运营', sub: '自媒体工厂' }
  if (platform.value === 'douyin') return { title: '抖音运营', sub: '自媒体工厂' }
  if (route.path.startsWith('/knowledge')) return { title: '知识库', sub: 'AI 内容工厂' }
  if (route.path.startsWith('/assets')) return { title: '素材中心', sub: 'AI 内容工厂' }
  return { title: '系统设置', sub: 'AI 内容工厂' }
})

const subNav = computed(() => {
  if (platform.value === 'xhs') {
    return [
      { path: '/xhs/accounts', label: '账号矩阵' },
      { path: '/xhs/search', label: '笔记发现' },
      { path: '/xhs/library', label: '内容库' },
      { path: '/xhs/competitors', label: '竞品分析' },
      { path: '/xhs/drafts', label: '草稿工坊' },
      { path: '/xhs/publish', label: '发布中心' },
      { path: '/xhs/auto-ops', label: '自动运营' },
    ]
  }
  if (platform.value === 'douyin') {
    return [
      { path: '/douyin/accounts', label: '账号矩阵' },
      { path: '/douyin/search', label: '作品发现' },
      { path: '/douyin/library', label: '内容库' },
      { path: '/douyin/drafts', label: '草稿工坊' },
      { path: '/douyin/publish', label: '发布中心' },
    ]
  }
  return []
})

function onPrimarySelect(index: string) {
  if (index === 'dashboard') router.push('/xhs/dashboard')
  else if (index === 'media-xhs') router.push('/xhs/accounts')
  else if (index === 'media-douyin') router.push('/douyin/accounts')
  else if (index === 'knowledge') router.push('/knowledge')
  else if (index === 'assets') router.push('/assets')
  else if (index === 'settings') router.push('/settings')
}

function goSub(path: string) {
  router.push(path)
}

function closeMediaSubmenu() {
  ignoreMediaToggle = true
  sideMenuRef.value?.close('media')
  queueMicrotask(() => {
    ignoreMediaToggle = false
  })
}

function openMediaSubmenu() {
  if (!mediaOpen.value || collapsed.value) return
  ignoreMediaToggle = true
  sideMenuRef.value?.open('media')
  queueMicrotask(() => {
    ignoreMediaToggle = false
  })
}

function syncMenuOpeneds(nextCollapsed: boolean) {
  if (openMediaTimer != null) {
    clearTimeout(openMediaTimer)
    openMediaTimer = null
  }
  if (nextCollapsed) {
    closeMediaSubmenu()
    return
  }
  // 先合上，等宽度过渡后再打开，避免子项瞬间撑高
  closeMediaSubmenu()
  if (!mediaOpen.value) return
  openMediaTimer = setTimeout(() => {
    openMediaTimer = null
    openMediaSubmenu()
  }, 220)
}

function toggleCollapse() {
  const next = !collapsed.value
  collapsed.value = next
  syncMenuOpeneds(next)
}

function onMenuOpen(index: string) {
  if (index === 'media' && !ignoreMediaToggle) mediaOpen.value = true
}

function onMenuClose(index: string) {
  if (index === 'media' && !ignoreMediaToggle) mediaOpen.value = false
}

watch(collapsed, (value) => {
  localStorage.setItem(COLLAPSE_KEY, value ? '1' : '0')
})

onMounted(() => {
  const isCollapsed = localStorage.getItem(COLLAPSE_KEY) === '1'
  collapsed.value = isCollapsed
  void nextTick(() => {
    if (!isCollapsed && mediaOpen.value) openMediaSubmenu()
  })
  // 后台竞品监控 / 今日最火挂在 globalThis，切页不停
  resumeCompetitorMonitor()
  resumeIndustryInsight()
})

onUnmounted(() => {
  if (openMediaTimer != null) clearTimeout(openMediaTimer)
})
</script>

<template>
  <div class="app-shell">
    <div class="app-body">
      <aside class="app-sidebar" :class="{ 'is-collapsed': collapsed }">
        <div class="app-brand">
          <img class="app-brand-logo" :src="brandLogo" alt="AI 内容工厂" />
          <div v-show="!collapsed" class="app-brand-text">
            <strong>AI 内容工厂</strong>
            <span>获客 · 内容运营</span>
          </div>
        </div>

        <el-menu
          ref="sideMenuRef"
          :default-active="primaryActive"
          :default-openeds="collapsed ? [] : mediaOpen ? ['media'] : []"
          :collapse="collapsed"
          :collapse-transition="false"
          class="side-menu"
          @select="onPrimarySelect"
          @open="onMenuOpen"
          @close="onMenuClose"
        >
          <el-menu-item index="dashboard">
            <el-icon><DataBoard /></el-icon>
            <span>仪表盘</span>
          </el-menu-item>
          <el-sub-menu index="media">
            <template #title>
              <el-icon><Document /></el-icon>
              <span>自媒体工厂</span>
            </template>
            <el-menu-item index="media-xhs">小红书运营</el-menu-item>
            <el-menu-item index="media-douyin">抖音运营</el-menu-item>
          </el-sub-menu>
          <el-menu-item index="knowledge">
            <el-icon><Cpu /></el-icon>
            <span>知识库</span>
          </el-menu-item>
          <el-menu-item index="assets">
            <el-icon><Picture /></el-icon>
            <span>素材中心</span>
          </el-menu-item>
          <el-menu-item index="settings">
            <el-icon><Setting /></el-icon>
            <span>系统设置</span>
          </el-menu-item>
        </el-menu>

        <button
          type="button"
          class="sidebar-toggle"
          :title="collapsed ? '展开菜单' : '折叠菜单'"
          @click="toggleCollapse"
        >
          <el-icon><DArrowRight v-if="collapsed" /><DArrowLeft v-else /></el-icon>
          <span v-show="!collapsed">折叠</span>
        </button>
      </aside>

      <div class="app-workspace">
        <div v-if="platform && !isDashboard" class="mf-ops">
          <aside class="mf-ops__sider">
            <div class="mf-ops__brand">
              <strong>{{ moduleTitle.title }}</strong>
              <span>{{ moduleTitle.sub }}</span>
            </div>
            <nav class="mf-ops__menu">
              <button
                v-for="item in subNav"
                :key="item.path"
                type="button"
                class="mf-ops__menu-btn"
                :class="{ 'is-active': route.path === item.path }"
                @click="goSub(item.path)"
              >
                <span>{{ item.label }}</span>
              </button>
            </nav>
          </aside>
          <main class="mf-ops__main">
            <router-view />
          </main>
        </div>

        <main
          v-else
          class="settings-main"
          :class="{
            'is-fill':
              isDashboard ||
              route.path.startsWith('/knowledge') ||
              route.path.startsWith('/assets'),
            'is-dash-scroll': isDashboard,
          }"
        >
          <div
            v-if="
              !isDashboard &&
              !route.path.startsWith('/knowledge') &&
              !route.path.startsWith('/assets')
            "
            class="settings-head"
          >
            <el-icon :size="18"><Cpu /></el-icon>
            <div>
              <strong>{{ moduleTitle.title }}</strong>
              <p>{{ moduleTitle.sub }}</p>
            </div>
          </div>
          <router-view />
        </main>
      </div>
    </div>
    <footer class="app-footer">
      <span
        v-if="analyzeBusy"
        class="app-footer__busy"
        :title="analyzeBusyTitle || '爆款分析'"
      >
        爆款分析进行中…
      </span>
      <span class="app-footer__copy">
        Copyright © 2025-2026 丛茂科技 All Rights Reserved.
      </span>
    </footer>
  </div>
</template>

<style scoped lang="scss">
.app-shell {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
  background: var(--app-bg);
}

.app-body {
  display: flex;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.app-footer {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 16px;
  min-height: 28px;
  padding: 6px 16px;
  border-top: 1px solid var(--line);
  background: var(--panel, #fff);
  color: var(--muted, #8a919f);
  font-size: 12px;
  line-height: 1.4;
  letter-spacing: 0.01em;
  user-select: none;
  position: relative;
}

.app-footer__busy {
  position: absolute;
  left: 16px;
  color: var(--el-color-primary, #409eff);
  font-weight: 500;
}

.app-footer__copy {
  text-align: center;
  color: #000;
}

.app-sidebar {
  width: 232px;
  flex: none;
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
  background: var(--sidebar-bg);
  border-right: 1px solid var(--line);
  padding: 14px;
  transition: width 0.2s ease;
  /* 过渡期间禁止出现滚动条闪现 */
  scrollbar-width: none;

  &::-webkit-scrollbar {
    display: none;
  }

  &.is-collapsed {
    width: 72px;
    padding-left: 10px;
    padding-right: 10px;
  }
}

.app-brand {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  flex: none;
  padding: 4px 6px 14px;
  margin-bottom: 4px;
  border-bottom: 1px solid var(--line);
}

.app-sidebar.is-collapsed .app-brand {
  justify-content: center;
  padding-left: 0;
  padding-right: 0;
}

.app-brand-logo {
  width: 44px;
  height: 44px;
  flex: none;
  border-radius: 10px;
  display: block;
  object-fit: contain;
  background: transparent;
}

.app-sidebar.is-collapsed .app-brand-logo {
  width: 48px;
  height: 48px;
}

.app-brand-text {
  min-width: 0;

  strong {
    display: block;
    font-size: 14px;
    font-weight: 700;
    color: var(--text);
    line-height: 1.3;
  }

  span {
    display: block;
    margin-top: 2px;
    font-size: 12px;
    color: var(--muted);
  }
}

.side-menu {
  flex: 1;
  min-height: 0;
  min-width: 0;
  border-right: none !important;
  background: transparent !important;
  /* 宽度动画时文字先展开、侧栏宽度滞后，隐藏横向滚动条避免闪一下 */
  overflow-x: hidden;
  overflow-y: auto;
  margin-top: 8px;
  scrollbar-width: none;
  -ms-overflow-style: none;

  &::-webkit-scrollbar {
    width: 0;
    height: 0;
    display: none;
  }

  &:not(.el-menu--collapse) {
    width: 100%;
  }

  :deep(.el-menu-item),
  :deep(.el-sub-menu__title) {
    border-radius: 10px;
    height: 42px !important;
    line-height: 42px !important;
    margin: 2px 0;
    overflow: hidden;
    box-sizing: border-box;
  }

  /* 折叠态也锁死同一高度，避免展开瞬间从 EP 默认高度跳到 42px */
  &.el-menu--collapse {
    width: 100%;

    :deep(.el-menu-item),
    :deep(.el-sub-menu__title) {
      height: 42px !important;
      line-height: 42px !important;
      padding: 0 !important;
      display: flex;
      justify-content: center;
      align-items: center;
    }

    /* EP 折叠态 tooltip 触发器默认左右 padding，导致图标偏左 */
    :deep(.el-menu-tooltip__trigger) {
      padding: 0 !important;
      justify-content: center;
      width: 100%;
    }

    :deep(.el-icon) {
      margin: 0 !important;
    }

    :deep(.el-sub-menu__icon-arrow) {
      display: none;
    }
  }

  :deep(.el-menu-item.is-active) {
    background: var(--primary-soft) !important;
    color: var(--primary) !important;
    font-weight: 600;
  }

  :deep(.el-sub-menu .el-menu-item) {
    min-width: 0;
    padding-left: 48px !important;
  }

  :deep(.el-sub-menu__title),
  :deep(.el-menu-item) {
    span {
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
  }
}

.sidebar-toggle {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  width: 100%;
  height: 36px;
  margin-top: 10px;
  padding: 0 8px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: var(--soft);
  color: var(--muted);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
  flex: none;
  box-sizing: border-box;
  overflow: hidden;

  span {
    white-space: nowrap;
  }

  &:hover {
    color: var(--text);
    border-color: var(--primary);
    background: var(--primary-soft);
  }
}

.app-sidebar.is-collapsed .sidebar-toggle {
  padding: 0;
}

.app-workspace {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  background: var(--panel);
  overflow: hidden;
}

.settings-main {
  flex: 1;
  min-width: 0;
  overflow: auto;
  padding: 20px 24px;
  background: var(--panel);
  display: flex;
  flex-direction: column;

  &.is-fill {
    padding: 0;
    overflow: hidden;

    > :deep(*) {
      flex: 1;
      min-height: 0;
    }
  }

  /* 仪表盘：内容按自然高度展开，整页滚动，避免增速等模块被压缩 */
  &.is-fill.is-dash-scroll {
    overflow: auto;

    > :deep(*) {
      flex: 0 0 auto;
      min-height: 100%;
      height: auto;
    }
  }
}

.settings-head {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  margin-bottom: 16px;
  color: var(--primary);

  strong {
    display: block;
    color: var(--text);
    font-size: 18px;
  }

  p {
    margin: 4px 0 0;
    color: var(--muted);
    font-size: 13px;
  }
}
</style>
