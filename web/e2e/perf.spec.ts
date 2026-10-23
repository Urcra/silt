import { execFileSync } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test } from '@playwright/test'
import { SILT, allThreads, fixture } from './fixture'

// `web.thread` of ADR 0034: a 2,000-message thread shows in under 100 ms
// and keeps the page under 200 MB. Timed on the optimized build the gate
// passes in `$SILT`; a debug build only reports. A cloud session's machine
// has its own limit, chosen as silt-bench chooses it (ADR 0037).
const ENFORCED = process.env.SILT !== undefined
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
    lines.push(JSON.stringify({ type: 'user', sessionId: session, cwd, message: { role: 'user', content: `prompt ${round}: look at the module` } }))
    lines.push(
      JSON.stringify({
        type: 'assistant',
        sessionId: session,
        cwd,
        message: { role: 'assistant', content: [{ type: 'tool_use', id: tool, name: 'Read', input: { file_path: `src/f${round}.rs` } }] },
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
  const row = page.getByTestId('threads').getByRole('listitem').filter({ hasText: thread })
  await expect(async () => {
    await page.reload()
    await expect(row).toContainText(`${2 * ROUNDS} messages`, { timeout: 1000 })
  }).toPass({ timeout: 60_000 })
  // Timed in the page, from the click to the first entry being drawn, as
  // Playwright's own work before a click lands takes tens of milliseconds.
  await page.evaluate(() => {
    const timed = window as unknown as Timed
    document.addEventListener('click', () => (timed.clicked = performance.now()), { capture: true, once: true })
    const drawn = new MutationObserver(() => {
      if (document.querySelector('[data-testid="history"]')?.textContent?.includes('prompt 0: look at the module')) {
        drawn.disconnect()
        requestAnimationFrame(() => (timed.shown = performance.now()))
      }
    })
    drawn.observe(document.body, { childList: true, subtree: true, characterData: true })
  })
  const history = page.waitForResponse((response) => response.url().includes('/history'))
  await link.click()
  const answered = await history
  console.log(`web.thread: the first history page is ${(await answered.body()).length} bytes`)
  await expect(page.getByTestId('history').getByText('prompt 0: look at the module')).toBeVisible({ timeout: 60_000 })
  const shown = await page.evaluate(() => {
    const { clicked, shown } = window as unknown as Timed
    return (shown ?? Number.NaN) - (clicked ?? Number.NaN)
  })
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('HeapProfiler.collectGarbage')
  const { usedSize } = (await cdp.send('Runtime.getHeapUsage')) as { usedSize: number }
  const heapMb = usedSize / 1024 / 1024
  console.log(`web.thread: shown in ${shown.toFixed(0)} ms (budget ${SHOWN_BUDGET_MS}), heap ${heapMb.toFixed(0)} MB (budget ${HEAP_BUDGET_MB})`)
  if (ENFORCED) {
    expect(shown, 'web.thread: shown').toBeLessThan(SHOWN_BUDGET_MS)
    expect(heapMb, 'web.thread: heap').toBeLessThan(HEAP_BUDGET_MB)
  }
})
