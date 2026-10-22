// The screen references (ADR 0035): every screen, light and dark, at desktop
// and phone width, compared with the screenshot kept beside this file. A
// deliberate change rewrites them with `pnpm e2e:update`, and review then
// compares the new one with its board.

import { execFileSync } from 'node:child_process'
import { readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { type Page, expect, test } from '@playwright/test'

const SILT = join(import.meta.dirname, '..', '..', 'target', 'debug', 'silt')

interface World {
  alice: string
  bob: string
  laptop: string
  server: string
  thread: string
  fork: string
  work: string
}

let world: World

test.beforeAll(() => {
  const output = execFileSync(join(import.meta.dirname, 'screens-fixture.sh'), { encoding: 'utf8' })
  world = JSON.parse(output.trim().split('\n').pop() ?? '{}') as World
})

test.afterAll(() => {
  for (const home of ['alice', 'bob', 'alice-laptop']) {
    execFileSync(SILT, ['daemon', 'stop'], { env: { ...process.env, SILT_HOME: join(world.work, home, 'silt') } })
  }
  process.kill(Number(readFileSync(join(world.work, 'server.pid'), 'utf8')))
  rmSync(world.work, { recursive: true, force: true })
})

const VARIANTS = [
  { name: 'desktop-light', width: 1440, height: 900, colorScheme: 'light' },
  { name: 'desktop-dark', width: 1440, height: 900, colorScheme: 'dark' },
  { name: 'phone-light', width: 390, height: 844, colorScheme: 'light' },
  { name: 'phone-dark', width: 390, height: 844, colorScheme: 'dark' },
] as const

/** Signs in to Alice's daemon and opens `path` once the app has settled. */
async function open(page: Page, path: string) {
  const origin = world.alice.split('#')[0].replace(/\/$/, '')
  await page.goto(world.alice)
  await page.goto(`${origin}${path}`)
  await page.waitForLoadState('networkidle')
}

/**
 * Takes the screen in every variant. Ids, hashes and times differ on each
 * run, so what shows them is marked `data-volatile` and masked.
 */
async function shoot(page: Page, name: string, ready: (page: Page) => Promise<void>) {
  for (const variant of VARIANTS) {
    await page.setViewportSize({ width: variant.width, height: variant.height })
    await page.emulateMedia({ colorScheme: variant.colorScheme, reducedMotion: 'reduce' })
    await ready(page)
    await expect(page).toHaveScreenshot(`${name}-${variant.name}.png`, {
      mask: [page.locator('[data-volatile]')],
      animations: 'disabled',
      caret: 'hide',
      maxDiffPixelRatio: 0.01,
    })
  }
}

test('the workspace home', async ({ page }) => {
  await open(page, '/')
  await shoot(page, 'home', async (shown) => {
    await expect(shown.getByRole('main')).toBeVisible()
  })
})

test('a thread', async ({ page }) => {
  await open(page, `/t/${world.thread}`)
  await shoot(page, 'thread', async (shown) => {
    await expect(shown.getByText('Please add a farewell line to main').first()).toBeVisible()
  })
})

test('a merge request', async ({ page }) => {
  await open(page, '/mrs')
  await page.getByRole('link', { name: 'Retry blob uploads on 5xx' }).first().click()
  await shoot(page, 'merge-request', async (shown) => {
    await expect(shown.getByTestId('mr-status')).toBeVisible()
  })
})
