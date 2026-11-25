import { execFileSync } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test } from '@playwright/test'
import { SILT, allThreads, fixture } from './fixture'

// `web.thread` of ADR 0034: a 2,000-message thread shows in under 100 ms
// and keeps the page under 200 MB. Timed on the optimized build the gate
// passes in `$SILT`; a debug build only reports. A cloud session's machine
// has its own limit, chosen as silt-bench chooses it (ADR 0037).
const ENFORCED = process.env.SILT !== undefined && process.env.SILT_BENCH_ENFORCE !== '0'
const CLOUD = (process.env.SILT_BENCH_MACHINE ?? (process.env.CLAUDE_CODE_REMOTE === 'true' ? 'cloud' : 'dev')) === 'cloud'
const SHOWN_BUDGET_MS = CLOUD ? 210 : 100
const HEAP_BUDGET_MB = 200
const ROUNDS = 1000

let thread = ''

/** When the thread's link was clicked and its first entry drawn, in the page's clock. */
interface Timed {
  clicked?: number
  shown?: number
}

/** A long session of prompts, tool calls with large results, and replies. */
function transcript(cwd: string, session: string): string {
  const lines: string[] = []
  const result = 'x'.repeat(15_000)
  for (let round = 0; round < ROUNDS; round++) {
    const tool = `toolu_${round}`
    lines.push(
      JSON.stringify({
        type: 'user',
        sessionId: session,
        cwd,
        message: { role: 'user', content: `prompt ${round}: look at the module` },
      }),
    )
    lines.push(
      JSON.stringify({
        type: 'assistant',
        sessionId: session,
        cwd,
        message: {
          role: 'assistant',
          content: [{ type: 'tool_use', id: tool, name: 'Read', input: { file_path: `src/f${round}.rs` } }],
        },
      }),
    )
    lines.push(
      JSON.stringify({
        type: 'user',
        sessionId: session,
        cwd,
        message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: tool, content: result }] },
      }),
    )
    lines.push(
      JSON.stringify({
        type: 'assistant',
        sessionId: session,
        cwd,
        message: { role: 'assistant', content: [{ type: 'text', text: `reply ${round}: changed it` }] },
      }),
    )
  }
  return lines.join('\n') + '\n'
}

/** The thread's last entry, which it opens at. */
const LAST_REPLY = `reply ${ROUNDS - 1}: changed it`

test.beforeAll(() => {
  const { work, siltHome } = fixture()
  const repo = join(work, 'long')
  mkdirSync(join(repo, 'src'), { recursive: true })
  writeFileSync(join(repo, 'src', 'main.rs'), 'fn main() {}\n')
  const git = (...args: string[]) => execFileSync('git', args, { cwd: repo })
  git('init', '-q', '-b', 'main')
  git('add', '.')
  git('-c', 'user.name=t', '-c', 'user.email=t@example.com', 'commit', '-q', '-m', 'init')
  const claude = join(work, 'claude')
  const session = '22222222-3333-4444-8555-666666666666'
  const project = join(claude, 'projects', repo.replace(/[^a-zA-Z0-9]/g, '-'))
  mkdirSync(project, { recursive: true })
  writeFileSync(join(project, `${session}.jsonl`), transcript(repo, session))
  const env = { ...process.env, SILT_HOME: siltHome, CLAUDE_CONFIG_DIR: claude }
  const tracked = execFileSync(SILT, ['track'], { cwd: repo, encoding: 'utf8', env })
  thread = tracked.trim().split(/\s+/).pop() ?? ''
})

test('a 2,000-message thread shows quickly and stays small', async ({ page }) => {
  test.setTimeout(180_000)
  const { url } = fixture()
  await page.goto(url)
  await allThreads(page)
  const link = page.getByRole('link', { name: thread })
  // The whole session is recorded once the listing counts all its messages.
  const row = page.getByTestId('threads').getByRole('row').filter({ hasText: thread })
  await expect(async () => {
    await page.reload()
    await expect(row).toContainText(`${2 * ROUNDS} messages`, { timeout: 1000 })
  }).toPass({ timeout: 60_000 })
  // Timed in the page, from the click to its last entry being drawn, as it
  // opens at its end (ADR 0117), and Playwright's own work before a click
  // lands takes tens of milliseconds.
  await page.evaluate((last) => {
    const timed = window as unknown as Timed
    document.addEventListener('click', () => (timed.clicked = performance.now()), { capture: true, once: true })
    const drawn = new MutationObserver(() => {
      if (document.querySelector('[data-testid="history"]')?.textContent?.includes(last)) {
        drawn.disconnect()
        requestAnimationFrame(() => (timed.shown = performance.now()))
      }
    })
    drawn.observe(document.body, { childList: true, subtree: true, characterData: true })
  }, LAST_REPLY)
  const history = page.waitForResponse((response) => response.url().includes('/history'))
  await link.click()
  const answered = await history
  console.log(`web.thread: the first history page is ${(await answered.body()).length} bytes`)
  await expect(page.getByTestId('history').getByText(LAST_REPLY)).toBeVisible({ timeout: 60_000 })
  const shown = await page.evaluate(() => {
    const { clicked, shown } = window as unknown as Timed
    return (shown ?? Number.NaN) - (clicked ?? Number.NaN)
  })
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('HeapProfiler.collectGarbage')
  const { usedSize } = (await cdp.send('Runtime.getHeapUsage')) as { usedSize: number }
  const heapMb = usedSize / 1024 / 1024
  console.log(
    `web.thread: shown in ${shown.toFixed(0)} ms (budget ${SHOWN_BUDGET_MS}), heap ${heapMb.toFixed(0)} MB (budget ${HEAP_BUDGET_MB})`,
  )
  if (ENFORCED) {
    expect(shown, 'web.thread: shown').toBeLessThan(SHOWN_BUDGET_MS)
    expect(heapMb, 'web.thread: heap').toBeLessThan(HEAP_BUDGET_MB)
  }
})

/** Opens the long thread and waits for its whole history, the page landed at its end. */
async function openLong(page: import('@playwright/test').Page) {
  const { url } = fixture()
  await page.goto(url)
  await allThreads(page)
  await page.getByRole('link', { name: thread }).click()
  await expect(page.getByTestId('history').getByText(LAST_REPLY)).toBeVisible({ timeout: 60_000 })
}

/** The main thread's long tasks from here on, as the page reports them. */
async function watchLongTasks(page: import('@playwright/test').Page) {
  await page.evaluate(() => {
    const held = window as unknown as { longTasks: number[] }
    held.longTasks = []
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) held.longTasks.push(entry.duration)
    }).observe({ type: 'longtask' })
  })
  return () => page.evaluate(() => (window as unknown as { longTasks: number[] }).longTasks)
}

// `web.scroll` (ADR 0101): scrolling the 2,000-message thread draws only the rows coming into view, so the page
// keeps answering. Each scroll drew the whole page and parsed every row's Markdown again: 13 s blocked over 150
// wheel notches on a long session.
const SCROLL_BLOCKED_MS = CLOUD ? 1000 : 500

test('a long thread scrolls without holding the page', async ({ page }) => {
  test.setTimeout(180_000)
  await openLong(page)
  const history = page.getByTestId('history')
  const box = await history.boundingBox()
  await page.mouse.move((box?.x ?? 0) + (box?.width ?? 0) / 2, 400)
  const longTasks = await watchLongTasks(page)
  for (let notch = 0; notch < 100; notch++) {
    await page.mouse.wheel(0, -600)
    await page.waitForTimeout(16)
  }
  await page.waitForTimeout(300)
  const blocked = (await longTasks()).reduce((total, duration) => total + duration, 0)
  console.log(`web.scroll: ${blocked.toFixed(0)} ms blocked over 100 notches (budget ${SCROLL_BLOCKED_MS})`)
  if (ENFORCED) expect(blocked, 'web.scroll: blocked').toBeLessThan(SCROLL_BLOCKED_MS)
})

// `web.change` (ADR 0101): a change to an open thread reads only what changed, not its whole history again, and a
// page of history carries a long output only from its end.
test('a change to an open thread reads only what changed', async ({ page }) => {
  test.setTimeout(180_000)
  await openLong(page)
  await page.waitForLoadState('networkidle')
  let read = 0
  page.on('response', (response) => {
    if (!response.url().includes('/history')) return
    void response.body().then(
      (body) => (read += body.length),
      () => undefined,
    )
  })
  const heard = page.waitForResponse((response) => response.url().includes('/comments') && response.request().method() === 'GET')
  await page.evaluate(async (id) => {
    const token = sessionStorage.getItem('silt-token')
    await fetch(`/api/threads/${id}/comments`, {
      method: 'POST',
      headers: { Authorization: `Bearer ${token}`, 'X-Silt-Request': '1', 'Content-Type': 'application/json' },
      body: JSON.stringify({ on: { on: 'entry', index: 3 }, text: 'a comment' }),
    })
  }, thread)
  // The comment is heard as a change to the thread, which the page reads again.
  await heard
  await page.waitForTimeout(1000)
  await page.waitForLoadState('networkidle')
  console.log(`web.change: ${read} bytes of history read again after one change`)
  expect(read, 'web.change: history read again').toBeLessThan(64 * 1024)
  const first = await page.evaluate(async (id) => {
    const token = sessionStorage.getItem('silt-token')
    const page = await fetch(`/api/threads/${id}/history?offset=0&limit=50`, { headers: { Authorization: `Bearer ${token}` } })
    return (await page.text()).length
  }, thread)
  // 25 tool outputs of 15 KB each, carried 4 KB at most.
  expect(first, "a page of history carries long outputs' ends").toBeLessThan(200 * 1024)
})

test('typing in the composer tells the others once it pauses, not at each key', async ({ page }) => {
  test.setTimeout(180_000)
  await openLong(page)
  await page.waitForLoadState('networkidle')
  let told = 0
  page.on('request', (request) => {
    if (request.url().includes('/presence')) told++
  })
  await page.getByRole('textbox').last().click()
  await page.keyboard.type('a prompt typed at a steady pace, key by key', { delay: 30 })
  await page.waitForTimeout(500)
  expect(told, 'presence requests while typing').toBeLessThanOrEqual(4)
})
