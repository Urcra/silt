import { execFileSync } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test } from '@playwright/test'
import { SILT, fixture } from './fixture'

// `web.thread` of ADR 0034: a 2,000-message thread shows in under 100 ms
// and keeps the page under 200 MB. Pending until the thread page loads its
// history in pages (M18-06), so it reports rather than fails until then.
const ENFORCED = false
const SHOWN_BUDGET_MS = 100
const HEAP_BUDGET_MB = 200
const ROUNDS = 1000

let thread = ''

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
  const link = page.getByRole('link', { name: thread })
  // The whole session is recorded once the listing counts all its messages.
  const row = page.getByTestId('threads').getByRole('row').filter({ hasText: thread })
  await expect(async () => {
    await page.reload()
    await expect(row).toContainText(`${2 * ROUNDS}`, { timeout: 1000 })
  }).toPass({ timeout: 60_000 })
  const started = Date.now()
  const history = page.waitForResponse((response) => response.url().includes('/history'))
  await link.click()
  const answered = await history
  console.log(`web.thread: history answered after ${Date.now() - started} ms, ${(await answered.body()).length} bytes`)
  await expect(page.getByTestId('history').getByText('prompt 0: look at the module')).toBeVisible({ timeout: 60_000 })
  const shown = Date.now() - started
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('HeapProfiler.collectGarbage')
  const { usedSize } = (await cdp.send('Runtime.getHeapUsage')) as { usedSize: number }
  const heapMb = usedSize / 1024 / 1024
  console.log(`web.thread: shown in ${shown} ms (budget ${SHOWN_BUDGET_MS}), heap ${heapMb.toFixed(0)} MB (budget ${HEAP_BUDGET_MB})`)
  if (ENFORCED) {
    expect(shown, 'web.thread: shown').toBeLessThan(SHOWN_BUDGET_MS)
    expect(heapMb, 'web.thread: heap').toBeLessThan(HEAP_BUDGET_MB)
  }
})
