// The busy thread's screen reference (ADR 0039): one Claude Code session
// with a pasted screenshot, commands and their output, a multiple-choice
// question answered, a plan, edits and a screenshot back, as a thread shows it,
// grouped by run and by turn (ADR 0049).

import { join } from 'node:path'
import { type Page, expect, test } from '@playwright/test'
import { runFixture, teardown } from './fixture'

const SILT = join(import.meta.dirname, '..', '..', 'target', 'debug', 'silt')

let world: { url: string; thread: string; work: string }

test.beforeAll(() => {
  world = JSON.parse(runFixture('busy-fixture.sh') || '{}') as typeof world
})

test.afterAll(() => {
  teardown(SILT, world.work, [join(world.work, 'silt')])
})

/** Picks how the transcript groups the agent's work, or how much each step shows, from the palette (Settings › Text
 * and layout keeps the same choice). */
async function choose(page: Page, group: 'transcript grouping' | 'transcript detail', label: string) {
  await page.keyboard.press('ControlOrMeta+k')
  const palette = page.getByRole('dialog', { name: 'Command palette' })
  await palette
    .getByLabel('command')
    .fill(group === 'transcript grouping' ? `Group the work ${label.toLowerCase()}` : `Transcript: ${label}`)
  await page.keyboard.press('Enter')
  await expect(palette).toBeHidden()
  // No hover or focus left over from the menu in the shot.
  await page.mouse.move(1430, 1790)
  await page.waitForTimeout(300)
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
}

test('a busy thread', async ({ page }) => {
  await page.goto(world.url)
  await page.goto(`${new URL(world.url).origin}/t/${world.thread}`)
  await page.setViewportSize({ width: 1440, height: 1800 })
  await choose(page, 'transcript grouping', 'By run')
  for (const colorScheme of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme, reducedMotion: 'reduce' })
    await expect(page.getByTestId('tools').first()).toBeVisible()
    // The failed command stands out of its run.
    await expect(page.getByTestId('command').first()).toContainText('failed')
    await expect(page.getByTestId('question')).toContainText('answered')
    await expect(page.getByTestId('plan-steps')).toContainText('3 of 3 done')
    await expect(page.getByText(/^Fixed: progress now counts/)).toBeVisible()
    await expect(page).toHaveScreenshot(`busy-thread-desktop-${colorScheme}.png`, {
      mask: [page.locator('[data-volatile]'), page.getByText(/\/tmp\/silt-|127\.0\.0\.1:\d+/).filter({ visible: true })],
      animations: 'disabled',
      caret: 'hide',
      maxDiffPixelRatio: 0.001,
    })
  }
})

test('a busy thread, by turn and verbose', async ({ page }) => {
  await page.goto(world.url)
  await page.goto(`${new URL(world.url).origin}/t/${world.thread}`)
  await page.setViewportSize({ width: 1440, height: 1800 })
  await page.emulateMedia({ colorScheme: 'dark', reducedMotion: 'reduce' })
  await choose(page, 'transcript grouping', 'By turn')
  await expect(page.getByTestId('worked')).toContainText('1 failed on the way')
  await expect(page.getByTestId('command').first()).toBeHidden()
  await expect(page.getByText(/^Fixed: progress now counts/)).toBeVisible()
  const shot = {
    mask: [page.locator('[data-volatile]')],
    animations: 'disabled' as const,
    caret: 'hide' as const,
    maxDiffPixelRatio: 0.001,
  }
  await expect(page).toHaveScreenshot('busy-thread-by-turn-dark.png', shot)
  await choose(page, 'transcript detail', 'Verbose')
  await expect(page.getByTestId('thought').first()).toContainText('The bar reaches 100% early')
  await expect(page.getByText('test result: FAILED. 1 passed; 1 failed').first()).toBeVisible()
  await expect(page).toHaveScreenshot('busy-thread-verbose-dark.png', shot)
  await choose(page, 'transcript detail', 'Normal')
})

test('Shift+T opens every thought, and again closes them', async ({ page }) => {
  await page.goto(world.url)
  await page.goto(`${new URL(world.url).origin}/t/${world.thread}`)
  await page.getByText(/^Fixed: progress now counts/).waitFor()
  const thought = page.getByText('The bar reaches 100% early')
  await expect(thought).toBeHidden()
  await page.keyboard.press('Shift+T')
  await expect(thought).toBeVisible()
  await page.keyboard.press('Shift+T')
  await expect(thought).toBeHidden()
})
