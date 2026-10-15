import { createRouter, createWebHistory } from 'vue-router'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', redirect: '/xhs/dashboard' },
    {
      path: '/xhs/dashboard',
      name: 'xhs-dashboard',
      component: () => import('@/views/xhs/Dashboard.vue'),
      meta: { platform: 'xhs', title: '仪表盘' },
    },
    {
      path: '/xhs/accounts',
      name: 'xhs-accounts',
      component: () => import('@/views/xhs/Accounts.vue'),
      meta: { platform: 'xhs', title: '账号矩阵' },
    },
    {
      path: '/xhs/search',
      name: 'xhs-search',
      component: () => import('@/views/xhs/Search.vue'),
      meta: { platform: 'xhs', title: '笔记发现' },
    },
    {
      path: '/xhs/library',
      name: 'xhs-library',
      component: () => import('@/views/xhs/Library.vue'),
      meta: { platform: 'xhs', title: '内容库' },
    },
    {
      path: '/xhs/competitors',
      name: 'xhs-competitors',
      component: () => import('@/views/xhs/Competitors.vue'),
      meta: { platform: 'xhs', title: '竞品分析' },
    },
    {
      path: '/xhs/drafts',
      name: 'xhs-drafts',
      component: () => import('@/views/xhs/Drafts.vue'),
      meta: { platform: 'xhs', title: '草稿工坊' },
    },
    {
      path: '/xhs/publish',
      name: 'xhs-publish',
      component: () => import('@/views/xhs/Publish.vue'),
      meta: { platform: 'xhs', title: '发布中心' },
    },
    {
      path: '/xhs/auto-ops',
      name: 'xhs-auto-ops',
      component: () => import('@/views/xhs/AutoOps.vue'),
      meta: { platform: 'xhs', title: '自动运营' },
    },
    {
      path: '/douyin/accounts',
      name: 'douyin-accounts',
      component: () => import('@/views/douyin/Accounts.vue'),
      meta: { platform: 'douyin', title: '账号矩阵' },
    },
    {
      path: '/douyin/search',
      name: 'douyin-search',
      component: () => import('@/views/douyin/Search.vue'),
      meta: { platform: 'douyin', title: '作品发现' },
    },
    {
      path: '/douyin/library',
      name: 'douyin-library',
      component: () => import('@/views/douyin/Library.vue'),
      meta: { platform: 'douyin', title: '内容库' },
    },
    {
      path: '/douyin/drafts',
      name: 'douyin-drafts',
      component: () => import('@/views/douyin/Drafts.vue'),
      meta: { platform: 'douyin', title: '草稿工坊' },
    },
    {
      path: '/douyin/publish',
      name: 'douyin-publish',
      component: () => import('@/views/douyin/Publish.vue'),
      meta: { platform: 'douyin', title: '发布中心' },
    },
    {
      path: '/knowledge',
      name: 'knowledge',
      component: () => import('@/views/KnowledgeBase.vue'),
      meta: { title: '知识库' },
    },
    {
      path: '/assets',
      name: 'assets',
      component: () => import('@/views/AssetCenter.vue'),
      meta: { title: '素材中心' },
    },
    {
      path: '/settings',
      name: 'settings',
      component: () => import('@/views/Settings.vue'),
      meta: { title: '系统设置' },
    },
  ],
})

export default router
