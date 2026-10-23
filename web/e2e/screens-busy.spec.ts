// The busy thread's screen reference (ADR 0039): one Claude Code session
// with a pasted screenshot, commands and their output, a multiple-choice
// question answered, edits and a screenshot back, as a thread shows it.

import { execFileSync } from 'node:child_process'
import { rmSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test } from '@playwright/test'

const SILT = join(import.meta.dirname, '..', '..', 'target', 'debug', 'silt')

let world: { url: string; thread: string; work: string }

test.beforeAll(() => {
  const output = execFileSync(join(import.meta.dirname, 'busy-fixture.sh'), { encoding: 'utf8' })
  world = JSON.parse(output.trim().split('\n').pop() ?? '{}') as typeof world
})

test.afterAll(() => {
  execFileSync(SILT, ['daemon', 'stop'], { env: { ...process.env, SILT_HOME: join(world.work, 'silt') } })
  rmSync(world.work, { recursive: true, force: true })
})

test('a busy thread', async ({ page }) => {
  await page.goto(world.url)
  await page.goto(`${new URL(world.url).origin}/t/${world.thread}`)
  await page.setViewportSize({ width: 1440, height: 1800 })
  for (const colorScheme of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme, reducedMotion: 'reduce' })
    await expect(page.getByTestId('tools')).toBeVisible()
    await expect(page.getByText(/^Fixed: progress now counts/)).toBeVisible()
    await expect(page).toHaveScreenshot(`busy-thread-desktop-${colorScheme}.png`, {
      mask: [page.locator('[data-volatile]'), page.getByText(/\/tmp\/silt-|127\.0\.0\.1:\d+/).filter({ visible: true })],
      animations: 'disabled',
      caret: 'hide',
      maxDiffPixelRatio: 0.001,
    })
  }
})
