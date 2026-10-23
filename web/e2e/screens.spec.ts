// The screen references (ADR 0039): every screen, light and dark, at desktop
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
  const origin = new URL(world.alice).origin
  await page.goto(world.alice)
  await page.goto(`${origin}${path}`)
  await page.waitForLoadState('networkidle')
}

/**
 * Ids, hashes and times differ on each run, so what shows them is marked
 * `data-volatile` and masked, as are the fixture's temporary paths and ports.
 * The tolerance is tight enough that a changed word fails.
 */
function shotOptions(page: Page) {
  return {
    mask: [page.locator('[data-volatile]'), page.getByText(/\/tmp\/silt-|127\.0\.0\.1:\d+|forked from [0-9a-f]+/).filter({ visible: true })],
    animations: 'disabled' as const,
    caret: 'hide' as const,
    maxDiffPixelRatio: 0.001,
  }
}

/** Takes the screen in every variant. */
async function shoot(page: Page, name: string, ready: (page: Page) => Promise<void>) {
  for (const variant of VARIANTS) {
    await page.setViewportSize({ width: variant.width, height: variant.height })
    await page.emulateMedia({ colorScheme: variant.colorScheme, reducedMotion: 'reduce' })
    await ready(page)
    await expect(page).toHaveScreenshot(`${name}-${variant.name}.png`, shotOptions(page))
  }
}

test('the workspace home, a new thread (ADR 0044)', async ({ page }) => {
  await open(page, '/')
  await shoot(page, 'home', async (shown) => {
    await expect(shown.getByRole('form', { name: 'new thread' })).toBeVisible()
  })
})

test('all threads', async ({ page }) => {
  await open(page, '/threads')
  await shoot(page, 'threads', async (shown) => {
    await expect(shown.getByTestId('threads')).toBeVisible()
  })
})

test('the command palette', async ({ page }) => {
  await open(page, '/')
  await shoot(page, 'palette', async (shown) => {
    await shown.keyboard.press('Escape')
    await shown.getByRole('button', { name: 'command palette' }).click()
    await shown.getByLabel('command', { exact: true }).fill('farewell')
    await expect(shown.getByText('In history, including before compaction')).toBeVisible()
  })
})

test('switching workspace', async ({ page }) => {
  await open(page, '/')
  await shoot(page, 'switcher', async (shown) => {
    await shown.keyboard.press('Escape')
    await shown.keyboard.press('Escape')
    const drawer = shown.getByRole('button', { name: 'show projects' })
    if ((shown.viewportSize()?.width ?? 0) < 768 && (await drawer.count()) > 0) await drawer.click()
    await shown.getByRole('button', { name: /^workspace: / }).click()
    await expect(shown.getByRole('dialog', { name: 'Switch workspace' })).toBeVisible()
  })
})

test('a thread', async ({ page }) => {
  await open(page, `/t/${world.thread}`)
  await shoot(page, 'thread', async (shown) => {
    await expect(shown.getByText('Please add a farewell line to main').first()).toBeVisible()
  })
})

test("a thread's details", async ({ page }) => {
  await open(page, `/t/${world.thread}`)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.getByRole('tab', { name: 'Details' }).click()
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme, reducedMotion: 'reduce' })
    await expect(page.getByTestId('code-comments')).toBeVisible()
    await expect(page).toHaveScreenshot(`thread-details-desktop-${colorScheme}.png`, shotOptions(page))
  }
})

test('a live session', async ({ page }) => {
  await open(page, `/t/${world.thread}/live`)
  await shoot(page, 'live', async (shown) => {
    const file = shown.getByRole('button', { name: /^src\/main\.rs/ })
    if ((await file.getAttribute('aria-current')) !== 'true') await file.click()
    await expect(shown.getByTestId('editor')).toContainText('fn farewell() {}')
  })
})

test('sharing a thread', async ({ page }) => {
  await open(page, `/t/${world.thread}`)
  await shoot(page, 'share', async (shown) => {
    if ((await shown.getByRole('dialog', { name: /^Share / }).count()) === 0) {
      await shown.getByRole('button', { name: 'Share' }).click()
    }
    await expect(shown.getByRole('dialog', { name: /^Share / }).getByRole('list', { name: 'collaborators' })).toBeVisible()
  })
})

test('reconciling a diverged thread', async ({ page }) => {
  // A real divergence needs a restored backup; the page only reads which
  // thread recovered into which, so the daemon's status names the fork.
  await page.route('**/api/status', async (route) => {
    const response = await route.fetch()
    const status = (await response.json()) as Record<string, unknown>
    const threads = (await (await route.fetch({ url: route.request().url().replace(/status$/, 'threads') })).json()) as { id: string }[]
    const full = (short: string) => threads.find((thread) => thread.id.endsWith(short))?.id ?? short
    status.recoveries = [{ thread: full(world.thread), recovery: full(world.fork), time_ms: Date.now() - 3_600_000 }]
    await route.fulfill({ response, json: status })
  })
  await open(page, `/t/${world.fork}/reconcile`)
  await shoot(page, 'reconcile', async (shown) => {
    await expect(shown.getByText(/differed from the server’s/).first()).toBeVisible()
  })
})

test("a new thread's project picker", async ({ page }) => {
  await shoot(page, 'new-thread', async (shown) => {
    await open(shown, '/?pick=projects')
    await expect(shown.getByRole('dialog', { name: 'Pick a project' })).toBeVisible()
  })
})

test('needs you', async ({ page }) => {
  await open(page, '/needs-you')
  await shoot(page, 'needs-you', async (shown) => {
    await expect(shown.getByRole('region', { name: 'waiting on you' }).getByRole('group').first()).toBeVisible()
  })
})

test('devices and pairing a phone', async ({ page }) => {
  await open(page, '/settings/devices')
  await shoot(page, 'devices', async (shown) => {
    await expect(shown.getByRole('region', { name: 'pair a phone' })).toBeVisible()
    await expect(shown.getByText('this device', { exact: true })).toBeVisible()
  })
})

test('projects', async ({ page }) => {
  await open(page, '/settings/projects')
  await shoot(page, 'projects', async (shown) => {
    await expect(shown.getByTestId('projects')).toBeVisible()
  })
})

test('adding a project', async ({ page }) => {
  await open(page, '/settings/add')
  await shoot(page, 'add-project', async (shown) => {
    await expect(shown.getByRole('region', { name: 'add a project' })).toBeVisible()
  })
})

test("the server's sign-in page", async ({ page }) => {
  await page.goto(`${world.server}/login`)
  await shoot(page, 'sign-in', async (shown) => {
    await expect(shown.getByRole('button', { name: 'Sign in' })).toBeVisible()
  })
})

test('search', async ({ page }) => {
  // Only the prompt: a server's hit is placed by time, and the reply and
  // summary are recorded in the compaction's millisecond, either side of it.
  await open(page, '/search?q=farewell%20line')
  await shoot(page, 'search', async (shown) => {
    await expect(shown.getByTestId('search-results')).toBeVisible()
  })
})

test('the inbox', async ({ page }) => {
  await open(page, '/inbox')
  await shoot(page, 'inbox', async (shown) => {
    await expect(shown.getByRole('tablist', { name: 'inbox filter' })).toBeVisible()
  })
})

test('the history graph', async ({ page }) => {
  await open(page, '/graph')
  await shoot(page, 'graph', async (shown) => {
    await expect(shown.getByRole('list', { name: 'graph' })).toBeVisible()
  })
})

test('merge requests', async ({ page }) => {
  await open(page, '/mrs')
  await shoot(page, 'merge-requests', async (shown) => {
    await expect(shown.getByTestId('merge-requests')).toBeVisible()
  })
})

test('a merge request', async ({ page }) => {
  await open(page, '/mrs')
  await page.getByRole('link', { name: 'Retry blob uploads on 5xx' }).first().click()
  await shoot(page, 'merge-request', async (shown) => {
    await expect(shown.getByTestId('mr-status')).toBeVisible()
  })
})
