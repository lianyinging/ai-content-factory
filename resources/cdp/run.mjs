#!/usr/bin/env node
import { mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { chromium } from 'playwright-core'
import * as humanize from './humanize.mjs'

const XHS_PUBLISH_URL = 'https://creator.xiaohongshu.com/publish/publish?source=official'
const XHS_HOME = 'https://creator.xiaohongshu.com/new/home'
const DOUYIN_HOME = 'https://creator.douyin.com/'
const DOUYIN_PUBLISH_URL = 'https://creator.douyin.com/creator-micro/content/upload'

/** 实时进度行：Rust 边读 stdout 边 emit 到前端滚动日志 */
function emitCdpProgress(step, draftId = '') {
  try {
    process.stdout.write(
      `${JSON.stringify({
        __cdpProgress: true,
        step: String(step),
        draftId: String(draftId || ''),
      })}\n`,
    )
  } catch {
    /* ignore */
  }
}

/** steps.push 时同步推送进度（fillXhsTopicChips 等共用同一数组） */
function createProgressSteps(draftId = '') {
  const steps = []
  const rawPush = steps.push.bind(steps)
  steps.push = (...items) => {
    for (const item of items) emitCdpProgress(item, draftId)
    return rawPush(...items)
  }
  return steps
}

const PROBE_XHS_JS = `(() => {
  const textOf = (el) => (el && (el.textContent || el.innerText) || '').replace(/\\s+/g, ' ').trim();
  const bodyText = (document.body?.innerText || '').replace(/\\s+/g, ' ').trim();
  const loginHints = /扫码登录|手机号登录|登录后发布|请先登录|立即登录/;
  if (loginHints.test(bodyText) && /login/i.test(location.href)) {
    return {
      loggedIn: false, nickname: '', avatarUrl: '', redId: '',
      following: '', followers: '', likesCollect: ''
    };
  }
  const img =
    document.querySelector('img.avatar, img[class*="avatar"], .user-info img, .creator-info img, header img, .account-info img')
    || null;
  const nameEl =
    document.querySelector('.user-name, .nickname, .creator-name, .account-name, [class*="userName"], [class*="nickName"], [class*="user-name"]')
    || null;
  let nickname = textOf(nameEl);
  if (!nickname) {
    const m = bodyText.match(/你好[，,\\s]*([^\\s，,]{2,20})/);
    if (m) nickname = m[1];
  }
  const avatarUrl = img?.src || '';
  let redId = '';
  const redLabel = [...document.querySelectorAll('span, div, p, em, li')].find((n) => {
    const t = textOf(n);
    return /^(小红书号|红薯号)[:：]?/.test(t) || t.includes('小红书号');
  });
  if (redLabel) {
    const raw = textOf(redLabel);
    const m1 = raw.match(/(?:小红书号|红薯号)\\s*[:：]?\\s*([A-Za-z0-9_\\-]{3,32})/);
    if (m1) redId = m1[1];
    else {
      const sib = redLabel.nextElementSibling || redLabel.parentElement;
      const m2 = textOf(sib).match(/([A-Za-z0-9_\\-]{3,32})/);
      if (m2) redId = m2[1];
    }
  }
  if (!redId) {
    const m = bodyText.match(/(?:小红书号|红薯号)\\s*[:：]?\\s*([A-Za-z0-9_\\-]{3,32})/);
    if (m) redId = m[1];
  }
  const pickStat = (labels) => {
    for (const label of labels) {
      const nodes = [...document.querySelectorAll('span, div, p, em, li, strong, b')];
      for (const n of nodes) {
        const t = textOf(n);
        if (!t) continue;
        if (t === label || t.startsWith(label)) {
          const parent = n.parentElement;
          if (parent) {
            const kids = [...parent.children];
            const numKid = kids.find((c) => /^[\\d,.]+\\s*[万wW]?$/.test(textOf(c)));
            if (numKid) return textOf(numKid);
            const prev = n.previousElementSibling;
            if (prev && /^[\\d,.]+\\s*[万wW]?$/.test(textOf(prev))) return textOf(prev);
            const next = n.nextElementSibling;
            if (next && /^[\\d,.]+\\s*[万wW]?$/.test(textOf(next))) return textOf(next);
          }
          const mInline = t.match(new RegExp(label + '\\\\s*[:：]?\\\\s*([\\\\d,.]+\\\\s*[万wW]?)'));
          if (mInline) return mInline[1].trim();
        }
      }
      const mBody = bodyText.match(new RegExp('([\\\\d,.]+\\\\s*[万wW]?)\\\\s*' + label));
      if (mBody) return mBody[1].trim();
      const mBody2 = bodyText.match(new RegExp(label + '\\\\s*[:：]?\\\\s*([\\\\d,.]+\\\\s*[万wW]?)'));
      if (mBody2) return mBody2[1].trim();
    }
    return '';
  };
  const following = pickStat(['关注']);
  const followers = pickStat(['粉丝']);
  const likesCollect = pickStat(['获赞与收藏', '获赞和收藏', '赞与收藏', '获赞']);
  const loggedIn = !loginHints.test(bodyText.slice(0, 200)) && (
    !!nickname || !!avatarUrl || !!redId || !!followers ||
    /creator\\.xiaohongshu\\.com\\/(new|publish)/.test(location.href)
  );
  return {
    loggedIn: !!loggedIn,
    nickname: nickname || '',
    avatarUrl: avatarUrl || '',
    redId: redId || '',
    following: following || '',
    followers: followers || '',
    likesCollect: likesCollect || ''
  };
})()`

const PROBE_DOUYIN_JS = `(() => {
  const bodyText = (document.body?.innerText || '').replace(/\\s+/g, ' ').trim();
  const loginHints = /登录|扫码|验证码登录|抖音号登录/;
  const onLogin = /login|passport/i.test(location.href);
  if (onLogin && loginHints.test(bodyText)) {
    return { loggedIn: false, nickname: '', avatarUrl: '' };
  }
  const img = document.querySelector('img[class*="avatar"], .avatar img, header img');
  const nameEl = document.querySelector('[class*="name"], .user-name, .nickname');
  const nickname = (nameEl?.textContent || '').trim();
  const avatarUrl = img?.src || '';
  const loggedIn = !onLogin && (!!nickname || !!avatarUrl || /creator\\.douyin\\.com/.test(location.href));
  return { loggedIn: !!loggedIn && !(/请登录/.test(bodyText.slice(0, 80))), nickname, avatarUrl };
})()`

const DISMISS_POPOVER_JS = `(() => {
  document.querySelectorAll('div.d-popover').forEach((el) => el.remove());
  const nodes = [...document.querySelectorAll('button, span, div, a')];
  const gotIt = nodes.find((n) => {
    const t = (n.textContent || '').trim();
    return t === '我知道了' && n.children.length <= 2;
  });
  if (gotIt) { try { gotIt.click(); } catch (_) {} }
  document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  return { dismissed: !!gotIt };
})()`

const FIND_EDITORS_JS = `(() => {
  const visible = (el) => {
    if (!el) return false;
    const r = el.getBoundingClientRect();
    const st = window.getComputedStyle(el);
    return r.width > 2 && r.height > 2 && st.visibility !== 'hidden' && st.display !== 'none';
  };
  const pick = (sels) => {
    for (const s of sels) {
      const el = document.querySelector(s);
      if (el && visible(el)) return s;
    }
    for (const s of sels) {
      if (document.querySelector(s)) return s;
    }
    return null;
  };
  const titleSelector = pick([
    'div.d-input input', '.titleInput input', '.titleInput [contenteditable="true"]',
    '#title', 'input[placeholder*="标题"]', 'textarea[placeholder*="标题"]',
    '.title-input input', '[class*="title"] input',
  ]);
  const contentSelector = pick([
    '.ql-editor', '#quillEditor .ql-editor', '.tiptap.ProseMirror',
    '[contenteditable="true"][data-placeholder*="正文"]',
    '[contenteditable="true"][placeholder*="正文"]',
    '[role="textbox"]', 'div[contenteditable="true"]', 'textarea[placeholder*="正文"]',
  ]);
  const bodyText = (document.body?.innerText || '').replace(/\\s+/g, ' ').trim();
  return {
    titleSelector,
    contentSelector,
    maybeLogin: /登录|扫码|验证码/.test(bodyText) && !titleSelector,
  };
})()`

const FIND_DOUYIN_EDITORS_JS = `(() => {
  const visible = (el) => {
    if (!el) return false;
    const r = el.getBoundingClientRect();
    const st = window.getComputedStyle(el);
    return r.width > 2 && r.height > 2 && st.visibility !== 'hidden' && st.display !== 'none';
  };
  const pick = (sels) => {
    for (const s of sels) {
      const el = document.querySelector(s);
      if (el && visible(el)) return s;
    }
    for (const s of sels) {
      if (document.querySelector(s)) return s;
    }
    return null;
  };
  const titleSelector = pick([
    'input[placeholder*="标题"]', 'textarea[placeholder*="标题"]',
    '[class*="title"] input', '[class*="Title"] input',
    'input[maxlength="30"]', 'input[maxlength="55"]',
  ]);
  const contentSelector = pick([
    '[contenteditable="true"]', 'textarea[placeholder*="简介"]',
    'textarea[placeholder*="描述"]', 'textarea[placeholder*="作品"]',
    '.editor-kit-container', '[class*="editor"] [contenteditable="true"]',
  ]);
  const bodyText = (document.body?.innerText || '').replace(/\\s+/g, ' ').trim();
  return {
    titleSelector,
    contentSelector,
    maybeLogin: /登录|扫码|验证码/.test(bodyText) && !titleSelector && !document.querySelector('input[type="file"]'),
  };
})()`

async function readStdin() {
  const chunks = []
  for await (const c of process.stdin) chunks.push(c)
  const raw = Buffer.concat(chunks).toString('utf8').trim()
  if (!raw) return {}
  return JSON.parse(raw)
}

async function resolveImages(sources, tmp) {
  const out = []
  let i = 0
  for (const src of sources.slice(0, 18)) {
    const s = String(src || '').trim()
    if (!s) continue
    if (/^https?:\/\//i.test(s)) {
      const res = await fetch(s, { headers: { Referer: 'https://www.xiaohongshu.com/' } })
      if (!res.ok) throw new Error(`下载配图失败 HTTP ${res.status}`)
      const buf = Buffer.from(await res.arrayBuffer())
      const ext = (s.match(/\.(jpe?g|png|webp|gif|mp4|mov|webm|m4a|mp3)/i) || [, 'jpg'])[1]
      const path = join(tmp, `media_${i++}.${ext}`)
      writeFileSync(path, buf)
      out.push(path)
    } else {
      out.push(s)
    }
  }
  return out
}

/** Map draft/publish content type → creator header-tabs title (data-hp-kind suffix). */
function resolveXhsCreatorTab(payload, mediaPaths = []) {
  const raw = String(
    payload.mode ||
      payload.tab ||
      payload.contentType ||
      payload.content_type ||
      payload.noteType ||
      payload.note_type ||
      '',
  )
    .trim()
    .toLowerCase()

  const aliases = {
    上传视频: '上传视频',
    video: '上传视频',
    视频: '上传视频',
    上传图文: '上传图文',
    image: '上传图文',
    images: '上传图文',
    图文: '上传图文',
    note: '上传图文',
    写长文: '写长文',
    long: '写长文',
    article: '写长文',
    长文: '写长文',
    发播客: '发播客',
    podcast: '发播客',
    audio: '发播客',
    播客: '发播客',
  }
  if (raw && aliases[raw]) return aliases[raw]
  // Chinese exact / includes
  if (/上传视频|视频/.test(raw) && !/图文/.test(raw)) return '上传视频'
  if (/写长文|长文|article/.test(raw)) return '写长文'
  if (/发播客|播客|podcast|audio/.test(raw)) return '发播客'
  if (/上传图文|图文/.test(raw)) return '上传图文'

  const paths = mediaPaths.map((p) => String(p || '').toLowerCase())
  const hasVideo = paths.some((p) => /\.(mp4|mov|webm|m4v|avi|mkv)(\?|$)/i.test(p))
  const hasAudio = paths.some((p) => /\.(mp3|m4a|aac|wav|flac)(\?|$)/i.test(p))
  if (hasVideo) return '上传视频'
  if (hasAudio) return '发播客'
  return '上传图文'
}

/** Prefer publish/upload tab on host; else any host tab. Never invent a tab here. */
function findCreatorPage(browser, host, pathHints = []) {
  const h = String(host || '').toLowerCase()
  const hints = (pathHints || []).map((x) => String(x).toLowerCase()).filter(Boolean)
  let fallback = null
  for (const context of browser.contexts()) {
    for (const page of context.pages()) {
      const u = (page.url() || '').toLowerCase()
      if (!u.includes(h)) continue
      const hit = { context, page, found: true }
      if (hints.some((p) => u.includes(p))) return hit
      if (!fallback) fallback = hit
    }
  }
  return fallback || { context: browser.contexts()[0] || null, page: null, found: false }
}

function pageCoversUrl(current, target) {
  const cur = String(current || '').toLowerCase()
  const want = String(target || '').toLowerCase()
  if (!cur || !want) return false
  const hostOf = (u) => {
    try {
      return new URL(u).host.toLowerCase()
    } catch {
      return (u.split('://')[1] || u).split('/')[0].toLowerCase()
    }
  }
  if (hostOf(cur) !== hostOf(want) || !hostOf(want)) return false
  if (want.includes('/publish') || want.includes('/content/upload') || want.includes('/upload')) {
    return cur.includes('/publish') || cur.includes('/content/upload') || cur.includes('/upload')
  }
  if (want.includes('creator.xiaohongshu.com') || want.includes('creator.douyin.com')) {
    return true
  }
  const strip = (u) => u.split('#')[0].split('?')[0]
  return strip(cur) === strip(want) || cur.startsWith(strip(want))
}

/** Reuse existing tab when possible; only newPage when host tab is missing. */
async function ensureCreatorPage(browser, { host, url, pathHints = [] }) {
  let { context, page, found } = findCreatorPage(browser, host, pathHints)
  let openedNewTab = false
  if (!found || !page) {
    context = context || browser.contexts()[0] || (await browser.newContext())
    page = await context.newPage()
    openedNewTab = true
  }
  try {
    await page.bringToFront()
  } catch {
    /* ignore */
  }
  const cur = page.url() || ''
  let navigated = false
  if (openedNewTab || !pageCoversUrl(cur, url)) {
    await humanize.goto(page, url, { timeout: 60000 })
    navigated = true
  }
  return { context, page, openedNewTab, navigated, reused: found && !openedNewTab }
}

function pageUrls(browser) {
  const urls = []
  for (const context of browser.contexts()) {
    for (const page of context.pages()) {
      const u = (page.url() || '').trim()
      if (u) urls.push(u)
    }
  }
  return urls
}

async function smokeTest(payload) {
  const port = Number(payload.debugPort || payload.port || 0)
  if (!port) throw new Error('缺少 debugPort')
  const url = String(payload.url || 'https://example.com').trim()
  let browser
  try {
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
    const contexts = browser.contexts()
    const context = contexts[0] || (await browser.newContext())
    const page = context.pages()[0] || (await context.newPage())
    await humanize.beginSession(page)
    try {
      await humanize.goto(page, url, { timeout: 30000 })
      const title = await page.title()
      return {
        ok: true,
        url: page.url(),
        title,
        message: 'Playwright connect_over_cdp 成功',
      }
    } finally {
      await humanize.endSession(page)
    }
  } finally {
    try {
      if (browser) await browser.close()
    } catch {
      /* ignore */
    }
  }
}

async function probeSession(payload) {
  const port = Number(payload.debugPort || payload.port || 0)
  if (!port) throw new Error('缺少 debugPort')
  let platform = String(payload.platform || 'xhs').trim().toLowerCase()
  if (platform === 'xiaohongshu' || platform === '小红书') platform = 'xhs'
  if (platform === 'dy' || platform === '抖音') platform = 'douyin'
  if (platform !== 'xhs' && platform !== 'douyin') throw new Error('platform 须为 xhs 或 douyin')

  const creatorHost = platform === 'xhs' ? 'creator.xiaohongshu.com' : 'creator.douyin.com'
  const home = platform === 'xhs' ? XHS_HOME : DOUYIN_HOME
  const probeJs = platform === 'xhs' ? PROBE_XHS_JS : PROBE_DOUYIN_JS
  const platformLabel = platform === 'xhs' ? '小红书创作服务平台' : '抖音创作服务平台'

  let browser
  try {
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
    const tabUrls = pageUrls(browser)
    const ensured = await ensureCreatorPage(browser, {
      host: creatorHost,
      url: home,
      pathHints: platform === 'xhs' ? ['/new/home', '/publish', '/login'] : ['/creator', '/upload'],
    })
    const page = ensured.page
    const openedNewTab = ensured.openedNewTab
    const onCreator = ensured.reused

    await humanize.beginSession(page)
    try {
      await humanize.pause(page, 'read')

      const data = await page.evaluate(probeJs)
      const finalUrl = page.url() || ''
      const onCreatorNow = finalUrl.toLowerCase().includes(creatorHost)
      const loggedIn = Boolean(data?.loggedIn) && onCreatorNow
      const nickname = String(data?.nickname || '').trim()
      const avatarUrl = String(data?.avatarUrl || '').trim()
      const redId = String(data?.redId || '').trim()
      const following = String(data?.following || '').trim()
      const followers = String(data?.followers || '').trim()
      const likesCollect = String(data?.likesCollect || '').trim()

      let tip
      if (!onCreator && openedNewTab) tip = `原无${platformLabel}标签，已新开并打开首页`
      else if (onCreatorNow) tip = `已在${platformLabel}标签探测`
      else tip = `未处于${platformLabel}`

      let message
      if (loggedIn) message = `已登录 · ${tip}`
      else if (onCreatorNow) message = `未登录 · ${tip}`
      else message = `未检测到${platformLabel}，请先打开 ${creatorHost}`

      return {
        ok: true,
        platform,
        loggedIn,
        nickname,
        avatarUrl,
        redId,
        following,
        followers,
        likesCollect,
        url: finalUrl,
        onCreatorPlatform: onCreatorNow,
        hadCreatorTab: onCreator && !openedNewTab,
        openedNewTab,
        tabUrls: tabUrls.slice(0, 20),
        message,
      }
    } finally {
      await humanize.endSession(page)
    }
  } finally {
    try {
      if (browser) await browser.close()
    } catch {
      /* ignore */
    }
  }
}

/** 去掉正文里的 #话题（创作者后台话题应走 tippy「# 话题」，勿塞进正文） */
function stripContentHashtags(content) {
  let s = String(content || '')
    .replace(/\r\n/g, '\n')
  // 正文内任意位置的 #话题 一律剥掉（含上次误插到段中的残留）
  s = s.replace(/#[^\s#]+/gu, '')
  // 清理多余空白
  s = s
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .replace(/[ \t]{2,}/g, ' ')
    .trim()
  return s
}

/**
 * 正文填完后，平台常出现 .recommend-topic-wrapper .tag-group .tag
 * 若有推荐话题，随机点选若干个。
 */
async function pickRandomRecommendTopics(page, steps, { min = 2, max = 5 } = {}) {
  const wrapper = page.locator('.recommend-topic-wrapper').first()
  try {
    await wrapper.waitFor({ state: 'visible', timeout: 6000 })
  } catch {
    steps.push('recommend_topics:none')
    return
  }
  const tags = wrapper.locator('.tag-group .tag')
  const count = await tags.count()
  if (!count) {
    steps.push('recommend_topics:empty')
    return
  }
  const idxs = Array.from({ length: count }, (_, i) => i)
  for (let i = idxs.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1))
    ;[idxs[i], idxs[j]] = [idxs[j], idxs[i]]
  }
  const want = Math.min(
    count,
    Math.max(min, Math.floor(min + Math.random() * (max - min + 1))),
  )
  const picked = idxs.slice(0, want)
  const names = []
  for (const i of picked) {
    const el = tags.nth(i)
    const name = ((await el.innerText().catch(() => '')) || '').replace(/^#/, '').trim()
    try {
      await humanize.click(page, el, { timeout: 2500 })
      names.push(name || `#${i}`)
      await humanize.pause(page, 'micro')
    } catch {
      steps.push(`recommend_topic:fail:${name || i}`)
    }
  }
  steps.push(`recommend_topics:${names.length ? names.join('|') : '0'}`)
}

/** tippy 话题浮层：#creator-editor-topic-container .item > .name */
async function pickFromCreatorTopicTippy(page, tag) {
  const bare = String(tag || '').trim().replace(/^#/, '')
  if (!bare) return false
  const want = `#${bare}`
  const container = page.locator('#creator-editor-topic-container')
  try {
    await container.waitFor({ state: 'visible', timeout: 5000 })
  } catch {
    return false
  }
  const items = container.locator('.item')
  const count = await items.count()
  if (!count) return false

  // 1) .name 精确匹配 #tag
  for (let i = 0; i < count; i++) {
    const el = items.nth(i)
    const name = ((await el.locator('.name').innerText().catch(() => '')) || '').trim()
    const n = name.replace(/^#/, '')
    if (name === want || n === bare) {
      await humanize.click(page, el, { timeout: 2500 })
      return true
    }
  }
  // 2) 名称包含关键词（如搜「戒指」命中「#戒指推荐」时优选取浏览量高的第一项已由列表排序）
  for (let i = 0; i < count; i++) {
    const el = items.nth(i)
    const name = ((await el.locator('.name').innerText().catch(() => '')) || '').trim()
    if (name.includes(bare) || name.replace(/^#/, '').includes(bare)) {
      await humanize.click(page, el, { timeout: 2500 })
      return true
    }
  }
  // 3) 当前高亮 is-selected
  const selected = container.locator('.item.is-selected').first()
  if ((await selected.count()) > 0) {
    await humanize.click(page, selected, { timeout: 2500 })
    return true
  }
  // 4) 列表第一项
  await humanize.click(page, items.first(), { timeout: 2500 })
  return true
}

/**
 * 把光标强制移到正文编辑器末尾。
 * 不要点编辑器中心——会把光标落在段落中间，导致 #话题插到文中。
 */
async function moveCaretToContentEnd(page, editor) {
  const mod = process.platform === 'darwin' ? 'Meta' : 'Control'
  // 1) 点编辑器右下角（靠近文末），避免中心点击
  try {
    const box = await editor.boundingBox()
    if (box) {
      await page.mouse.click(box.x + Math.max(12, box.width - 16), box.y + Math.max(12, box.height - 12))
    }
  } catch {
    /* ignore */
  }
  // 2) JS / Quill / ProseMirror 选区塌到末尾
  await editor
    .evaluate((el) => {
      el.focus()
      el.scrollTop = el.scrollHeight
      try {
        const Quill = window.Quill
        if (Quill) {
          const q = Quill.find(el)
          if (q && typeof q.getLength === 'function' && typeof q.setSelection === 'function') {
            const len = q.getLength()
            q.setSelection(Math.max(0, len - 1), 0, 'user')
            q.focus()
            return 'quill'
          }
        }
      } catch {
        /* fall through */
      }
      try {
        // ProseMirror / Tiptap：找末尾文本节点
        const walk = document.createTreeWalker(el, NodeFilter.SHOW_TEXT, null)
        let last = null
        let n
        while ((n = walk.nextNode())) last = n
        const sel = window.getSelection()
        const range = document.createRange()
        if (last) {
          range.setStart(last, last.textContent ? last.textContent.length : 0)
          range.collapse(true)
        } else {
          range.selectNodeContents(el)
          range.collapse(false)
        }
        sel.removeAllRanges()
        sel.addRange(range)
        return 'range'
      } catch {
        return 'fail'
      }
    })
    .catch(() => {})
  // 3) 键盘兜底：Cmd/Ctrl+↓ → Cmd/Ctrl+→
  await page.keyboard.press(`${mod}+ArrowDown`).catch(() => {})
  await page.keyboard.press(`${mod}+ArrowRight`).catch(() => {})
  await page.keyboard.press('End').catch(() => {})
}

/**
 * 在正文编辑器末尾键入 #话题，弹出 tippy 后点选正式话题。
 * 对应 DOM：#creator-editor-topic-container .item .name
 */
async function fillXhsTopicChips(page, tags, steps, contentSelector) {
  const list = [...new Set(tags.map((t) => String(t).trim().replace(/^#/, '')).filter(Boolean))].slice(
    0,
    8,
  )
  if (!list.length) return
  const editor = page.locator(contentSelector || '.ql-editor, [contenteditable="true"]').first()
  let okCount = 0
  let startedTopicLine = false
  for (const tag of list) {
    try {
      await editor.waitFor({ state: 'visible', timeout: 4000 })
      // 每写一个标签前强制光标到文末（禁止点编辑器中心）
      await moveCaretToContentEnd(page, editor)
      // 只在正文与话题区之间换一次行；后续标签同一行用空格分隔，避免每个 # 各占一行
      if (!startedTopicLine) {
        await page.keyboard.press('Enter')
        startedTopicLine = true
      } else {
        await page.keyboard.type(' ', { delay: 20 })
      }
      await humanize.pause(page, 'micro')
      // 键入 #关键字 触发 tippy 浮层（不要整段粘贴 #tag 进正文）
      await page.keyboard.type(`#${tag}`, { delay: 45 })
      await humanize.pause(page, 'step')

      let picked = await pickFromCreatorTopicTippy(page, tag)
      if (!picked) {
        // 兜底：工具栏「话题」再开一次搜索
        const opened = await humanize.clickText(page, '话题', { exact: true, timeout: 2000 })
        if (opened) {
          await humanize.pause(page, 'step')
          await page.keyboard.type(tag, { delay: 40 }).catch(() => {})
          await humanize.pause(page, 'step')
          picked = await pickFromCreatorTopicTippy(page, tag)
        }
      }
      if (!picked) {
        // 取消未确认的 # 输入，避免正文残留
        await page.keyboard.press('Escape').catch(() => {})
        for (let i = 0; i < tag.length + 1; i++) {
          await page.keyboard.press('Backspace').catch(() => {})
        }
        steps.push(`topic:${tag}:fail`)
        continue
      }
      okCount += 1
      steps.push(`topic:${tag}:ok`)
      await humanize.pause(page, 'step')
      await page.keyboard.press('Escape').catch(() => {})
    } catch (e) {
      steps.push(`topic:${tag}:err:${String(e?.message || e).slice(0, 40)}`)
      await page.keyboard.press('Escape').catch(() => {})
    }
  }
  steps.push(`topics_filled:${okCount}/${list.length}`)
}

async function publishXhsNote(payload) {
  const port = Number(payload.debugPort || payload.port || 0)
  if (!port) throw new Error('缺少 debugPort')
  const title = String(payload.title || '').slice(0, 20)
  const tags = (payload.tags || []).map((t) => String(t).trim().replace(/^#/, '')).filter(Boolean)
  // 正文不再拼接 #标签；粘连的旧标签也先剥掉
  const content = stripContentHashtags(String(payload.content || payload.body || '').trim())
  const sources = [...(payload.imagePaths || []), ...(payload.imageUrls || [])]
  if (!sources.length) throw new Error('至少需要一张配图（本地路径或 URL）')

  const tmp = join(tmpdir(), `ai_xhs_pub_${Date.now()}`)
  mkdirSync(tmp, { recursive: true })
  const steps = createProgressSteps(payload.draftId)
  emitCdpProgress('cdp_script_start', payload.draftId)
  let browser
  try {
    const localImages = await resolveImages(sources, tmp)
    emitCdpProgress(`images_ready:${localImages.length}`, payload.draftId)
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
    const ensured = await ensureCreatorPage(browser, {
      host: 'creator.xiaohongshu.com',
      url: XHS_PUBLISH_URL,
      pathHints: ['/publish/publish', '/publish'],
    })
    const page = ensured.page
    steps.push(
      ensured.reused
        ? ensured.navigated
          ? 'reused_creator_tab:navigated_publish'
          : 'reused_publish_tab'
        : 'opened_publish_new_tab',
    )

    await humanize.beginSession(page)
    try {
      await page.evaluate(DISMISS_POPOVER_JS)
      await humanize.pause(page, 'step')

      // 按内容类型点 header-tabs：[data-hp-kind="creator-tab-上传图文|上传视频|写长文|发播客"]
      const modeTitle = resolveXhsCreatorTab(payload, [...sources, ...localImages])
      let tabHit = await humanize.clickCreatorTab(page, modeTitle, { timeout: 12000 })
      if (!tabHit.ok) {
        const fallback = await humanize.clickText(page, modeTitle, { exact: true, timeout: 4000 })
        tabHit = { ok: fallback, titles: tabHit.titles || [], title: modeTitle, kind: tabHit.kind }
      }
      steps.push(
        `creator_tab:${modeTitle}:${tabHit.ok}${tabHit.kind ? `[${tabHit.kind}]` : ''}${
          tabHit.titles?.length ? `(${tabHit.titles.join('|')})` : ''
        }`,
      )
      if (!tabHit.ok) {
        const seen = (tabHit.titles || []).join('、') || '无'
        throw new Error(
          `未找到发布模式 Tab「${modeTitle}」（data-hp-kind=creator-tab-${modeTitle}）。当前可见：${seen}`,
        )
      }
      await humanize.pause(page, 'step')

      for (const path of localImages) {
        await humanize.setInputFiles(page, 'input.upload-input, input[type="file"]', [path])
      }
      steps.push(`uploaded:${localImages.length}`)

      let editors = null
      for (let i = 0; i < 20; i++) {
        await page.evaluate(DISMISS_POPOVER_JS)
        editors = await page.evaluate(FIND_EDITORS_JS)
        if (editors?.maybeLogin) throw new Error('创作者中心未登录。请在自动化浏览器窗口内手动登录后重试。')
        if (editors?.titleSelector && editors?.contentSelector) break
        await humanize.pause(page, 'step')
      }
      if (!editors?.titleSelector || !editors?.contentSelector) {
        throw new Error('未找到标题/正文编辑器，请确认已进入图文发布页且上传完成')
      }

      await humanize.typeSelector(page, editors.titleSelector, title)
      steps.push('filled_title')
      await humanize.typeSelector(page, editors.contentSelector, content)
      steps.push('filled_content')

      // 推荐话题区：.recommend-topic-wrapper .tag-group .tag → 随机点选
      await humanize.pause(page, 'step')
      await pickRandomRecommendTopics(page, steps, { min: 2, max: 5 })

      if (tags.length) {
        await fillXhsTopicChips(page, tags, steps, editors.contentSelector)
      }

      // fill | saveDraft（暂存离开 .ce-btn.white）| publish（发布 .ce-btn.bg-red）
      const actionRaw = String(payload.action || '').trim().toLowerCase()
      let submitAction = 'fill'
      if (
        actionRaw === 'savedraft' ||
        actionRaw === 'save' ||
        actionRaw === 'draft' ||
        payload.saveDraft
      ) {
        submitAction = 'saveDraft'
      } else if (actionRaw === 'publish' || payload.publish) {
        submitAction = 'publish'
      }

      let published = false
      let savedDraft = false
      if (submitAction === 'publish') {
        const hit = await humanize.clickXhsPublishFooter(page, 'publish', { timeout: 10000 })
        steps.push(`click_publish:${hit.ok}`)
        await humanize.pause(page, 'read')
        let confirmed = await humanize.clickText(page, '确认发布', { exact: true, timeout: 3000 })
        if (!confirmed) confirmed = await humanize.clickText(page, '确定发布', { exact: true, timeout: 2000 })
        steps.push(`confirm:${confirmed}`)
        published = Boolean(hit.ok)
      } else if (submitAction === 'saveDraft') {
        const hit = await humanize.clickXhsPublishFooter(page, 'saveDraft', { timeout: 10000 })
        steps.push(`click_save_draft:${hit.ok}`)
        savedDraft = Boolean(hit.ok)
        if (!hit.ok) {
          throw new Error(
            '未找到「暂存离开」按钮（xhs-publish-btn closed Shadow DOM / .ce-btn.white）',
          )
        }
        await humanize.pause(page, 'read')
      }

      let message = '已填表到待发布状态，请在自动化浏览器中人工确认后发布'
      if (published) message = '已点击发布（请在浏览器中确认结果）'
      else if (savedDraft) message = '已点击「暂存离开」，内容已保存到小红书创作平台草稿'

      return {
        ok: true,
        draftId: payload.draftId || '',
        published,
        savedDraft,
        action: submitAction,
        url: page.url(),
        steps,
        message,
      }
    } finally {
      await humanize.endSession(page)
    }
  } finally {
    try {
      if (browser) await browser.close()
    } catch {
      /* ignore */
    }
    try {
      rmSync(tmp, { recursive: true, force: true })
    } catch {
      /* ignore */
    }
  }
}

async function publishDouyinNote(payload) {
  const port = Number(payload.debugPort || payload.port || 0)
  if (!port) throw new Error('缺少 debugPort')
  const title = String(payload.title || '').slice(0, 55)
  let content = String(payload.content || payload.body || '').trim()
  const tags = (payload.tags || []).map((t) => String(t).trim().replace(/^#/, '')).filter(Boolean)
  if (tags.length) {
    const tagLine = tags.map((t) => `#${t}`).join(' ')
    if (!content.includes(tagLine)) content = `${content}\n${tagLine}`
  }
  const sources = [...(payload.imagePaths || []), ...(payload.imageUrls || [])]
  if (!sources.length) throw new Error('至少需要一张配图或视频（本地路径或 URL）')

  const tmp = join(tmpdir(), `ai_dy_pub_${Date.now()}`)
  mkdirSync(tmp, { recursive: true })
  const steps = createProgressSteps(payload.draftId)
  emitCdpProgress('cdp_script_start', payload.draftId)
  let browser
  try {
    const localFiles = await resolveImages(sources, tmp)
    emitCdpProgress(`images_ready:${localFiles.length}`, payload.draftId)
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
    const ensured = await ensureCreatorPage(browser, {
      host: 'creator.douyin.com',
      url: DOUYIN_PUBLISH_URL,
      pathHints: ['/content/upload', '/upload'],
    })
    const page = ensured.page
    steps.push(
      ensured.reused
        ? ensured.navigated
          ? 'reused_creator_tab:navigated_upload'
          : 'reused_upload_tab'
        : 'opened_upload_new_tab',
    )

    await humanize.beginSession(page)
    try {
      await humanize.pause(page, 'step')

      // Prefer image/图文 entry when available
      let tab = await humanize.clickText(page, '上传图文', { exact: false, timeout: 5000 })
      if (!tab) tab = await humanize.clickText(page, '图文', { exact: true, timeout: 3000 })
      steps.push(`image_tab:${tab}`)
      await humanize.pause(page, 'step')

      for (const path of localFiles) {
        await humanize.setInputFiles(page, 'input[type="file"]', [path])
      }
      steps.push(`uploaded:${localFiles.length}`)

      let editors = null
      for (let i = 0; i < 24; i++) {
        editors = await page.evaluate(FIND_DOUYIN_EDITORS_JS)
        if (editors?.maybeLogin) {
          throw new Error('抖音创作者中心未登录。请在自动化浏览器窗口内手动登录后重试。')
        }
        if (editors?.titleSelector || editors?.contentSelector) break
        await humanize.pause(page, 'step')
      }
      if (editors?.titleSelector) {
        await humanize.typeSelector(page, editors.titleSelector, title)
        steps.push('filled_title')
      }
      if (editors?.contentSelector && content) {
        await humanize.typeSelector(page, editors.contentSelector, content)
        steps.push('filled_content')
      }
      if (!editors?.titleSelector && !editors?.contentSelector) {
        throw new Error('未找到标题/简介编辑器，请确认已进入抖音发布页且上传完成')
      }

      let published = false
      if (payload.publish) {
        let ok = await humanize.clickText(page, '发布', { exact: true, timeout: 8000 })
        if (!ok) ok = await humanize.clickText(page, '发布作品', { exact: false, timeout: 5000 })
        steps.push(`click_publish:${ok}`)
        published = Boolean(ok)
      }

      return {
        ok: true,
        draftId: payload.draftId || '',
        published,
        url: page.url(),
        steps,
        message: published
          ? '已点击发布（请在浏览器中确认结果）'
          : '已填表到待发布状态，请在自动化浏览器中人工确认后发布',
      }
    } finally {
      await humanize.endSession(page)
    }
  } finally {
    try {
      if (browser) await browser.close()
    } catch {
      /* ignore */
    }
    try {
      rmSync(tmp, { recursive: true, force: true })
    } catch {
      /* ignore */
    }
  }
}

const action = process.argv[2] || ''
const payload = await readStdin()

try {
  let out
  if (action === 'publish-xhs-note') {
    out = await publishXhsNote(payload)
  } else if (action === 'publish-douyin-note') {
    out = await publishDouyinNote(payload)
  } else if (action === 'probe-session') {
    out = await probeSession(payload)
  } else if (action === 'smoke') {
    out = await smokeTest(payload)
  } else {
    out = { ok: false, error: `unknown action: ${action}` }
  }
  console.log(JSON.stringify(out))
  process.exit(out.ok ? 0 : 1)
} catch (e) {
  console.log(JSON.stringify({ ok: false, error: e instanceof Error ? e.message : String(e) }))
  process.exit(1)
}
