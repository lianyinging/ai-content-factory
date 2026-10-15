/** Humanized Playwright actions — sole exit for CDP page mutations. */

const BANNER_ID = 'ss-ai-browser-banner'
const BANNER_TEXT = 'AI正在操作浏览器'

const BANNER_JS = `(() => {
  const ID = ${JSON.stringify(BANNER_ID)};
  let el = document.getElementById(ID);
  if (!el) {
    el = document.createElement('div');
    el.id = ID;
    el.setAttribute('data-ss-ai-banner', '1');
    el.textContent = ${JSON.stringify(BANNER_TEXT)};
    el.style.cssText = [
      'position:fixed','top:0','left:0','right:0','z-index:2147483647','pointer-events:none',
      'padding:8px 16px','font:600 13px/1.4 -apple-system,BlinkMacSystemFont,Segoe UI,sans-serif',
      'color:#fff','background:linear-gradient(90deg,#1a1a1a 0%,#c41e3a 100%)',
      'text-align:center','letter-spacing:0.02em','box-shadow:0 2px 8px rgba(0,0,0,.25)',
    ].join(';');
    (document.documentElement || document.body).appendChild(el);
  }
  el.style.display = 'block';
  return true;
})()`

const CLEAR_BANNER_JS = `(() => {
  const el = document.getElementById(${JSON.stringify(BANNER_ID)});
  if (el) el.remove();
  return true;
})()`

function rand(lo, hi) {
  return Math.floor(lo + Math.random() * (hi - lo + 1))
}

function ease(t) {
  return 1 - (1 - t) ** 1.35
}

function bezierPoint(t, p0, p1, p2, p3) {
  const u = 1 - t
  const x = u ** 3 * p0[0] + 3 * u ** 2 * t * p1[0] + 3 * u * t ** 2 * p2[0] + t ** 3 * p3[0]
  const y = u ** 3 * p0[1] + 3 * u ** 2 * t * p1[1] + 3 * u * t ** 2 * p2[1] + t ** 3 * p3[1]
  return [x, y]
}

function cubicPath(start, end) {
  const dx = end[0] - start[0]
  const dy = end[1] - start[1]
  const dist = Math.hypot(dx, dy)
  const n = Math.max(12, Math.min(48, Math.floor(dist / 12) + 8))
  const spread = Math.max(40, dist * 0.35)
  const c1 = [
    start[0] + dx * (0.15 + Math.random() * 0.25) + (Math.random() * 2 - 1) * spread * 0.4,
    start[1] + dy * (0.05 + Math.random() * 0.3) + (Math.random() * 2 - 1) * spread * 0.5,
  ]
  const c2 = [
    start[0] + dx * (0.55 + Math.random() * 0.3) + (Math.random() * 2 - 1) * spread * 0.4,
    start[1] + dy * (0.55 + Math.random() * 0.4) + (Math.random() * 2 - 1) * spread * 0.5,
  ]
  const pts = []
  for (let i = 1; i <= n; i++) {
    pts.push(bezierPoint(ease(i / n), start, c1, c2, end))
  }
  return pts
}

export async function pause(page, kind = 'step') {
  const ranges = {
    micro: [40, 120],
    step: [180, 520],
    read: [400, 1100],
    upload: [700, 1600],
    nav: [600, 1400],
  }
  const [lo, hi] = ranges[kind] || [200, 500]
  await page.waitForTimeout(rand(lo, hi))
}

export async function ensureAiBanner(page) {
  try {
    await page.evaluate(BANNER_JS)
  } catch {
    /* ignore */
  }
}

export async function clearAiBanner(page) {
  try {
    await page.evaluate(CLEAR_BANNER_JS)
  } catch {
    /* ignore */
  }
}

export async function beginSession(page) {
  await ensureAiBanner(page)
  await pause(page, 'micro')
}

export async function endSession(page) {
  await clearAiBanner(page)
}

export async function goto(page, url, { timeout = 60000 } = {}) {
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout })
  await ensureAiBanner(page)
  await pause(page, 'nav')
}

export async function moveTo(page, x, y) {
  const vp = page.viewportSize() || { width: 1280, height: 800 }
  let start = [vp.width * (0.2 + Math.random() * 0.6), vp.height * (0.2 + Math.random() * 0.5)]
  if (Array.isArray(page._ssMouse) && page._ssMouse.length === 2) start = page._ssMouse
  const path = cubicPath(start, [x, y])
  for (const [px, py] of path) {
    await page.mouse.move(px, py)
    await page.waitForTimeout(rand(4, 14))
  }
  page._ssMouse = [x, y]
}

async function targetPoint(locator) {
  const box = await locator.boundingBox()
  if (!box) throw new Error('元素不可见，无法拟人点击')
  const padX = Math.max(2, box.width * 0.2)
  const padY = Math.max(2, box.height * 0.2)
  const x = box.x + (padX + Math.random() * Math.max(1, box.width - 2 * padX))
  const y = box.y + (padY + Math.random() * Math.max(1, box.height - 2 * padY))
  return [x, y]
}

export async function click(page, locator, { timeout = 15000 } = {}) {
  await locator.waitFor({ state: 'visible', timeout })
  await locator.scrollIntoViewIfNeeded()
  await pause(page, 'micro')
  const [x, y] = await targetPoint(locator)
  await moveTo(page, x, y)
  await pause(page, 'micro')
  await page.mouse.down()
  await page.waitForTimeout(rand(40, 110))
  await page.mouse.up()
  page._ssMouse = [x, y]
  await pause(page, 'step')
  await ensureAiBanner(page)
}

export async function clickText(page, text, { exact = true, timeout = 15000 } = {}) {
  const loc = page.getByText(text, { exact }).first()
  try {
    await click(page, loc, { timeout })
    return true
  } catch {
    return false
  }
}

/**
 * Buttons live in closed Shadow DOM of <xhs-publish-btn>:
 *   .publish-page-publish-btn > .ce-btn.white「暂存离开」| .ce-btn.bg-red「发布」
 * Light DOM cannot query inside closed shadow — use a11y / CDP pierce / geometry.
 */
async function clickClosedShadowByText(page, text) {
  let session
  try {
    session = await page.context().newCDPSession(page)
    const search = await session.send('DOM.performSearch', {
      query: text,
      includeUserAgentShadowDOM: true,
    })
    if (!search?.resultCount) return false
    const { nodeIds } = await session.send('DOM.getSearchResults', {
      searchId: search.searchId,
      fromIndex: 0,
      toIndex: Math.min(search.resultCount, 8),
    })
    await session.send('DOM.discardSearchResults', { searchId: search.searchId }).catch(() => {})
    for (const nodeId of nodeIds || []) {
      try {
        await session.send('DOM.scrollIntoViewIfNeeded', { nodeId })
        const { model } = await session.send('DOM.getBoxModel', { nodeId })
        const quad = model?.content || model?.border
        if (!quad || quad.length < 8) continue
        const xs = [quad[0], quad[2], quad[4], quad[6]]
        const ys = [quad[1], quad[3], quad[5], quad[7]]
        const x = (Math.min(...xs) + Math.max(...xs)) / 2
        const y = (Math.min(...ys) + Math.max(...ys)) / 2
        await moveTo(page, x, y)
        await pause(page, 'micro')
        await page.mouse.down()
        await page.waitForTimeout(rand(40, 110))
        await page.mouse.up()
        page._ssMouse = [x, y]
        await pause(page, 'step')
        await ensureAiBanner(page)
        return true
      } catch {
        /* try next node */
      }
    }
  } catch {
    return false
  } finally {
    try {
      if (session) await session.detach()
    } catch {
      /* ignore */
    }
  }
  return false
}

/** Layout fallback: two 120px buttons, gap 24, centered in sticky host. Left=暂存, Right=发布 */
async function clickXhsPublishFooterByGeometry(page, want) {
  const host = page.locator('xhs-publish-btn').first()
  await host.waitFor({ state: 'visible', timeout: 4000 })
  await host.scrollIntoViewIfNeeded().catch(() => {})
  const box = await host.boundingBox()
  if (!box || box.width < 80 || box.height < 20) return false
  const btnW = 120
  const gap = 24
  const total = btnW * 2 + gap
  const startX = box.x + Math.max(0, (box.width - total) / 2)
  const cx = want === 'saveDraft' ? startX + btnW / 2 : startX + btnW + gap + btnW / 2
  const cy = box.y + box.height / 2
  await moveTo(page, cx, cy)
  await pause(page, 'micro')
  await page.mouse.down()
  await page.waitForTimeout(rand(40, 110))
  await page.mouse.up()
  page._ssMouse = [cx, cy]
  await pause(page, 'step')
  await ensureAiBanner(page)
  return true
}

export async function clickXhsPublishFooter(page, kind = 'publish', { timeout = 15000 } = {}) {
  const want = kind === 'saveDraft' ? 'saveDraft' : 'publish'
  const label = want === 'saveDraft' ? '暂存离开' : '发布'
  const deadline = Date.now() + Math.max(1000, timeout)
  const host = page.locator('xhs-publish-btn').first()

  while (Date.now() < deadline) {
    // 1) Host visible — try a11y role (often sees closed-shadow buttons)
    if (await host.isVisible().catch(() => false)) {
      const roleBtn = host.getByRole('button', { name: label }).first()
      try {
        if (await roleBtn.isVisible({ timeout: 800 }).catch(() => false)) {
          await click(page, roleBtn, { timeout: 4000 })
          return { ok: true, kind: want, via: 'role' }
        }
      } catch {
        /* continue */
      }
    }

    // 2) Page-level role / text (a11y tree)
    try {
      const btn = page.getByRole('button', { name: label }).first()
      if (await btn.isVisible({ timeout: 600 }).catch(() => false)) {
        await click(page, btn, { timeout: 4000 })
        return { ok: true, kind: want, via: 'role-page' }
      }
    } catch {
      /* continue */
    }

    // 3) CDP pierce closed shadow
    if (await clickClosedShadowByText(page, label)) {
      return { ok: true, kind: want, via: 'cdp' }
    }

    // 4) Geometry on <xhs-publish-btn>
    if (await host.isVisible().catch(() => false)) {
      if (await clickXhsPublishFooterByGeometry(page, want)) {
        return { ok: true, kind: want, via: 'geometry' }
      }
    }

    await pause(page, 'step')
  }
  return { ok: false, kind: want }
}

/**
 * Click a Xiaohongshu creator publish mode tab inside `.header-tabs`.
 * Prefer `[data-hp-kind="creator-tab-<title>"]` (上传视频 / 上传图文 / 写长文 / 发播客),
 * then fall back to `.creator-tab > span.title` text.
 */
export async function clickCreatorTab(page, title, { timeout = 12000 } = {}) {
  const want = String(title || '').trim()
  if (!want) return { ok: false, titles: [], kind: '' }

  const kind = `creator-tab-${want}`
  const deadline = Date.now() + Math.max(1000, timeout)
  let titles = []

  while (Date.now() < deadline) {
    // 1) data-hp-kind inside header-tabs (most stable)
    const byKind = page.locator(
      `.header-tabs [data-hp-kind="${kind}"], [data-hp-kind="${kind}"]`,
    )
    if ((await byKind.count().catch(() => 0)) > 0) {
      const target = byKind.first()
      if (await target.isVisible().catch(() => false)) {
        await click(page, target, { timeout: 5000 })
        return { ok: true, titles: [want], title: want, kind }
      }
    }

    // 2) .creator-tab by title text
    const tabs = page.locator('.header-tabs .creator-tab, .creator-tab')
    const n = await tabs.count().catch(() => 0)
    titles = []
    for (let i = 0; i < n; i++) {
      const tab = tabs.nth(i)
      const hpKind = (await tab.getAttribute('data-hp-kind').catch(() => '')) || ''
      const label = (
        (await tab.locator('span.title').first().textContent().catch(() => '')) ||
        (await tab.textContent().catch(() => '')) ||
        ''
      )
        .replace(/\s+/g, ' ')
        .trim()
      if (label) titles.push(label)
      else if (hpKind.startsWith('creator-tab-')) titles.push(hpKind.slice('creator-tab-'.length))

      const match =
        label === want ||
        hpKind === kind ||
        (label && want.includes(label)) ||
        (label && label.includes(want))
      if (match) {
        await click(page, tab, { timeout: 5000 })
        return { ok: true, titles, title: want, kind }
      }
    }
    await pause(page, 'step')
  }
  return { ok: false, titles, title: want, kind }
}

export async function typeText(page, locator, text, { clear = true } = {}) {
  await locator.waitFor({ state: 'visible', timeout: 15000 })
  await locator.scrollIntoViewIfNeeded()
  await click(page, locator)
  if (clear) {
    await page.keyboard.press('Meta+a')
    await pause(page, 'micro')
    await page.keyboard.press('Backspace')
    await pause(page, 'micro')
  }
  for (const ch of text) {
    await page.keyboard.type(ch, { delay: rand(35, 95) })
    if (Math.random() < 0.04) await pause(page, 'micro')
  }
  await pause(page, 'step')
  await ensureAiBanner(page)
}

export async function typeSelector(page, selector, text, opts) {
  await typeText(page, page.locator(selector).first(), text, opts)
}

export async function setInputFiles(page, selector, files) {
  await page.locator(selector).first().setInputFiles(files)
  await pause(page, 'upload')
  await ensureAiBanner(page)
}
