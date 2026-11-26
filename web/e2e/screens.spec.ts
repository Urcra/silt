// The screen references (ADR 0039): every screen, light and dark, at desktop
// and phone width, compared with the screenshot kept beside this file. A
// deliberate change rewrites them with `pnpm e2e:update`, and review then
// compares the new one with its board.

import { copyFileSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { type Page, expect, test } from '@playwright/test'
import { chip, pick, runFixture, teardown } from './fixture'

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
/** When the fixture began: the pages' clock stays there, so everything happened "just now" however slow the run. */
let started: number

test.beforeAll(() => {
  started = Date.now()
  world = JSON.parse(runFixture('screens-fixture.sh') || '{}') as World
})

test.afterAll(() => {
  const homes = ['alice', 'bob', 'alice-laptop'].map((home) => join(world.work, home, 'silt'))
  teardown(SILT, world.work, homes, [join(world.work, 'server.pid')])
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
  await page.clock.setFixedTime(started)
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
    mask: [
      page.locator('[data-volatile]'),
      page.getByText(/\/tmp\/silt-|\/silt-live-|127\.0\.0\.1:\d+|forked from [0-9a-f]+/).filter({ visible: true }),
    ],
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
    await expect(page).toHaveScreenshot(`${name}-${variant.name}.png`, {
      ...shotOptions(page),
      // The embedded type specimen rasterizes its own font slightly differently across machines.
      maxDiffPixelRatio: name === 'design-system-tokens' ? 0.002 : 0.001,
    })
  }
}

// First, on the fixture as it was made: later screens add threads to the sidebar.
/** Picks Alice's light and dark theme through her daemon, as Settings does. */
async function pickThemes(page: Page, settings: { mode: string; light: string; dark: string; text?: object }) {
  await page.evaluate(async (settings) => {
    const response = await fetch('/api/appearance', {
      method: 'PUT',
      headers: {
        Authorization: `Bearer ${sessionStorage.getItem('silt-token')}`,
        'Content-Type': 'application/json',
        'X-Silt-Request': '1',
      },
      body: JSON.stringify(settings),
    })
    if (!response.ok) throw new Error(await response.text())
  }, settings)
}

const NOUS = { mode: 'system', light: 'nous-light', dark: 'nous-dark' }

test('Settings › Appearance, and a thread in a bundled pair (ADR 0098)', async ({ page }) => {
  await open(page, '/settings/appearance')
  await shoot(page, 'appearance', async (shown) => {
    await expect(shown.getByRole('list', { name: 'themes' }).getByText('Catppuccin Mocha')).toBeVisible()
  })
  try {
    await pickThemes(page, { mode: 'system', light: 'catppuccin-latte', dark: 'catppuccin-mocha' })
    await open(page, `/t/${world.thread}`)
    await page.setViewportSize({ width: 1440, height: 900 })
    for (const colorScheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme, reducedMotion: 'reduce' })
      await expect(page.getByText('Please add a farewell line to main').first()).toBeVisible()
      await expect(page).toHaveScreenshot(`thread-catppuccin-desktop-${colorScheme}.png`, shotOptions(page))
    }
  } finally {
    await pickThemes(page, NOUS)
  }
})

test('Settings › Text and layout, and a thread at larger sizes (ADR 0107)', async ({ page }) => {
  await open(page, '/settings/text')
  await shoot(page, 'text-layout', async (shown) => {
    await expect(shown.getByRole('region', { name: 'preview' }).getByText('whitespace-pre-wrap').first()).toBeVisible()
  })
  const larger = { chat_size: 17, code_size: 14, ui_size: 15, density: 'roomy', line_height: 'loose', width: 'wide' }
  try {
    await pickThemes(page, { ...NOUS, text: larger })
    await open(page, '/settings/text')
    await shoot(page, 'text-layout-changed', async (shown) => {
      await expect(shown.getByRole('button', { name: 'reset conversation text' })).toBeVisible()
    })
    await open(page, `/t/${world.thread}`)
    await page.setViewportSize({ width: 1440, height: 900 })
    for (const colorScheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme, reducedMotion: 'reduce' })
      await expect(page.getByText('Please add a farewell line to main').first()).toBeVisible()
      await expect(page).toHaveScreenshot(`thread-larger-desktop-${colorScheme}.png`, shotOptions(page))
    }
  } finally {
    await pickThemes(page, NOUS)
  }
})

test('text and layout set in Settings reach the page, are kept, and reset (ADR 0107)', async ({ page }) => {
  await open(page, '/settings/text')
  const section = page.getByRole('region', { name: 'Text and layout' })
  const variable = (name: string) =>
    page.evaluate((name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim(), name)
  try {
    await section.getByRole('button', { name: 'bigger conversation text' }).click()
    await section.getByRole('button', { name: 'smaller code text' }).click()
    await section.getByRole('radiogroup', { name: 'density' }).getByRole('radio', { name: 'Compact' }).click()
    await section.getByRole('radiogroup', { name: 'reduce motion' }).getByRole('radio', { name: 'Always' }).click()
    await expect.poll(() => variable('--chat-size')).toBe('14px')
    await expect.poll(() => variable('--code-size')).toBe('11px')
    await expect.poll(() => variable('--row')).toBe('26px')
    await expect(page.locator('html')).toHaveAttribute('data-motion', 'reduced')
    // Kept by the daemon, and put in the page it serves, so the first frame is at the size set.
    const served = await page.request.get(`${new URL(world.alice).origin}/`)
    expect(await served.text()).toContain('--chat-size:14px;--code-size:11px;--row:26px;--density:0.6;')
    await open(page, `/t/${world.thread}`)
    const reply = page.getByText('Done: farewell added.').first()
    await expect(reply).toHaveCSS('font-size', '14px')
    await open(page, '/settings/text')
    await expect(section.locator('[data-modified]')).toHaveCount(4)
    await section.getByRole('button', { name: 'reset conversation text' }).click()
    await expect.poll(() => variable('--chat-size')).toBe('13px')
    await section.getByRole('button', { name: 'Reset all' }).click()
    await expect(section.locator('[data-modified]')).toHaveCount(0)
    await expect.poll(() => variable('--code-size')).toBe('12px')
    await expect(page.locator('html')).not.toHaveAttribute('data-motion', 'reduced')
  } finally {
    await pickThemes(page, NOUS)
  }
})

test("a theme's file edited while the page is open restyles it (ADR 0098)", async ({ page }) => {
  const themes = join(world.work, 'alice', 'silt', 'themes')
  mkdirSync(themes, { recursive: true })
  const file = join(themes, 'live-edit.json')
  const write = (bg: string) =>
    writeFileSync(
      file,
      JSON.stringify({
        name: 'Live',
        themes: [{ id: 'live-edit', name: 'Live', appearance: 'light', colors: { bg, 'fg-1': '#1b1b1b', accent: '#0055cc' } }],
      }),
    )
  write('#fafafa')
  await open(page, '/settings/appearance')
  await page.emulateMedia({ colorScheme: 'light' })
  const bg = () => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--bg').trim())
  try {
    await pickThemes(page, { ...NOUS, light: 'live-edit' })
    await expect.poll(bg).toBe('#fafafa')
    write('#eef4ff')
    await expect.poll(bg, { timeout: 10_000 }).toBe('#eef4ff')
  } finally {
    await pickThemes(page, NOUS)
    rmSync(file, { force: true })
  }
})

test('a theme that ships its own font loads it from its folder (ADR 0099)', async ({ page }) => {
  const root = join(world.work, 'alice', 'silt', 'themes', 'shipped')
  mkdirSync(join(root, 'fonts'), { recursive: true })
  copyFileSync(
    join(
      import.meta.dirname,
      '..',
      'node_modules',
      '@fontsource',
      'ibm-plex-mono',
      'files',
      'ibm-plex-mono-latin-100-italic.woff2',
    ),
    join(root, 'fonts', 'thin.woff2'),
  )
  writeFileSync(
    join(root, 'theme.json'),
    JSON.stringify({
      name: 'Shipped',
      themes: [
        {
          id: 'shipped',
          name: 'Shipped',
          appearance: 'light',
          colors: { bg: '#fafafa', 'fg-1': '#1b1b1b', accent: '#0055cc' },
          fonts: {
            ui: ['Shipped Mono', 'monospace'],
            faces: [{ family: 'Shipped Mono', file: 'fonts/thin.woff2', style: 'italic', weight: 100 }],
          },
        },
      ],
    }),
  )
  await open(page, '/settings/appearance')
  await page.emulateMedia({ colorScheme: 'light' })
  try {
    await pickThemes(page, { ...NOUS, light: 'shipped' })
    await page.reload()
    await page.waitForLoadState('networkidle')
    const loaded = await page.evaluate(async () => {
      await document.fonts.load('italic 100 16px "Shipped Mono"')
      return document.fonts.check('italic 100 16px "Shipped Mono"')
    })
    expect(loaded).toBe(true)
    const served = await page.request.get(`${new URL(world.alice).origin}/theme-fonts/shipped/thin.woff2`)
    expect(served.headers()['content-type']).toBe('font/woff2')
    expect((await page.request.get(`${new URL(world.alice).origin}/theme-fonts/shipped/..%2Ftheme.json`)).status()).toBe(404)
  } finally {
    await pickThemes(page, NOUS)
    rmSync(root, { recursive: true, force: true })
  }
})

test('the workspace home, a new thread (ADR 0044)', async ({ page }) => {
  await open(page, '/')
  await shoot(page, 'home', async (shown) => {
    // A phone's home is the list; its new thread opens from there.
    if ((shown.viewportSize()?.width ?? 0) < 768) return expect(shown.getByTestId('threads')).toBeVisible()
    await expect(shown.getByRole('form', { name: 'new thread' })).toBeVisible()
  })
})

test('a new chat (ADR 0059)', async ({ page }) => {
  await open(page, '/chat')
  await shoot(page, 'new-chat', async (shown) => {
    await expect(shown.getByRole('form', { name: 'new chat' })).toBeVisible()
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
    const drawer = shown.getByRole('button', { name: 'show projects' })
    if ((shown.viewportSize()?.width ?? 0) < 768 && (await drawer.count()) > 0) await drawer.click()
    await shown.getByRole('navigation', { name: 'projects' }).getByRole('button', { name: 'Search' }).click()
    await shown.getByLabel('command', { exact: true }).fill('farewell')
    await expect(shown.getByText('In history, including before compaction')).toBeVisible()
    // The group shows at once, with Search everything; its hits come once the search answers.
    await expect(shown.getByRole('option', { name: /^“/ }).first()).toBeVisible()
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

test("a thread's review, across the page (ADR 0067)", async ({ page }) => {
  // A phone reviews in the panel's sheet, so the review is a desktop's only.
  await page.setViewportSize({ width: 1440, height: 900 })
  await open(page, `/t/${world.thread}/review`)
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme, reducedMotion: 'reduce' })
    await expect(page.getByRole('region', { name: 'changes' }).getByTestId('diff')).toContainText('fn farewell() {}')
    await expect(page).toHaveScreenshot(`review-desktop-${colorScheme}.png`, shotOptions(page))
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
    const threads = (await (
      await route.fetch({
        url: route
          .request()
          .url()
          .replace(/status$/, 'threads'),
      })
    ).json()) as { id: string }[]
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
    const picker = shown.getByRole('dialog', { name: 'Pick a project' })
    await expect(picker).toBeVisible()
    // The page fills in as its lists load, each on its own, after the picker opens.
    await expect(picker.getByText('tools', { exact: true })).toBeVisible()
    const main = shown.getByRole('main')
    await expect(chip(main, 'agent to start')).toHaveAttribute('data-value', 'plain')
    await expect(main.getByText('Add a farewell')).toBeVisible()
    await expect(main.getByText('Retry blob uploads on a 5xx')).toBeVisible()
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
    await expect(shown.getByRole('region', { name: 'phones on this network' })).toBeVisible()
    await expect(shown.getByText('this device', { exact: true })).toBeVisible()
  })
  // Through the server, from anywhere (ADR 0028), beside pairing on this network (ADR 0112).
  await page.getByRole('button', { name: 'Through your server' }).click()
  await shoot(page, 'devices-server', async (shown) => {
    await expect(shown.getByRole('region', { name: 'pair a phone' })).toBeVisible()
  })
})

test('Settings › Network, online and offline (ADR 0134)', async ({ page }) => {
  await open(page, '/settings/network')
  await shoot(page, 'network', async (shown) => {
    await expect(shown.getByRole('switch', { name: 'Plan usage' })).toBeChecked()
  })
  await page.getByRole('switch', { name: 'Offline' }).click()
  await expect(page.getByRole('switch', { name: 'Offline' })).toBeChecked()
  // Clicked at the last variant's width, which scrolls the page; open it afresh, with nothing hovered.
  await open(page, '/settings/network')
  await page.mouse.move(0, 0)
  try {
    await shoot(page, 'network-offline', async (shown) => {
      await expect(shown.getByRole('switch', { name: 'Plan usage' })).toBeDisabled()
    })
  } finally {
    await page.getByRole('switch', { name: 'Offline' }).click()
    await expect(page.getByRole('switch', { name: 'Plan usage' })).toBeEnabled()
  }
})

test('projects', async ({ page }) => {
  await open(page, '/settings/projects')
  await shoot(page, 'projects', async (shown) => {
    await expect(shown.getByTestId('projects')).toBeVisible()
  })
})

test('design systems (ADR 0088)', async ({ page }) => {
  await open(page, '/settings/design-systems')
  await shoot(page, 'design-systems', async (shown) => {
    await expect(shown.getByRole('link', { name: /Harbor/ })).toBeVisible()
  })
})

test('a new design system (ADR 0088)', async ({ page }) => {
  await open(page, '/settings/design-systems')
  await shoot(page, 'new-design-system', async (shown) => {
    if (!(await shown.getByRole('dialog', { name: 'New design system' }).isVisible()))
      await shown.getByRole('button', { name: 'New design system' }).first().click()
    await shown.getByRole('textbox', { name: 'Name' }).fill('Harbor')
    await expect(shown.getByRole('button', { name: 'Start building' })).toBeVisible()
  })
})

test("a design system's page (ADR 0088)", async ({ page }) => {
  await open(page, '/settings/design-systems/system?source=personal&name=harbor')
  await shoot(page, 'design-system', async (shown) => {
    await expect(shown.frameLocator('iframe[title="button preview"]').getByRole('button', { name: 'Set sail' })).toBeVisible()
    await expect(shown.frameLocator('iframe[title="tag preview"]').getByText('Docked')).toBeVisible()
    // Each frame sizes itself to its content once drawn.
    await shown.waitForTimeout(300)
  })
})

test("a design system's tokens (ADR 0088)", async ({ page }) => {
  await open(page, '/settings/design-systems/system?source=personal&name=harbor')
  await shoot(page, 'design-system-tokens', async (shown) => {
    await shown.getByRole('tab', { name: 'Tokens' }).click()
    await expect(
      shown.frameLocator('iframe[title="type scale"]').getByText('Sphinx of black quartz, judge my vow').first(),
    ).toBeVisible()
    await shown.waitForTimeout(300)
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

test('the first open, with no projects yet', async ({ page }) => {
  // A fresh machine has no clones; what the agents left on disk is found.
  await page.route('**/api/agents', async (route) => {
    const response = await route.fetch()
    await route.fulfill({ response, json: { ...((await response.json()) as object), clones: [] } })
  })
  await page.route('**/api/found', (route) =>
    route.fulfill({
      json: [
        {
          path: '/home/alice/src/api',
          repo: 'github.com/acme/api',
          git: true,
          added: false,
          sessions: [{ agent: 'Claude Code', agent_id: 'claude', count: 12 }],
          last_ms: Date.now() - 7_200_000,
        },
        {
          path: '/home/alice/src/site',
          repo: 'github.com/acme/site',
          git: true,
          added: false,
          sessions: [{ agent: 'Codex', agent_id: 'codex', count: 3 }],
          last_ms: Date.now() - 86_400_000 * 3,
        },
        {
          path: '/home/alice/notes',
          repo: '',
          git: false,
          added: false,
          sessions: [{ agent: 'Claude Code', agent_id: 'claude', count: 2 }],
          last_ms: Date.now() - 86_400_000 * 9,
        },
      ],
    }),
  )
  await shoot(page, 'first-open', async (shown) => {
    await open(shown, (shown.viewportSize()?.width ?? 0) < 768 ? '/new' : '/')
    await expect(shown.getByRole('heading', { name: 'Pick a folder to start in' })).toBeVisible()
  })
})

// Last: these start agents, which every screen after them would show.
async function startFake(page: Page, prompt: string, preset?: string) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await open(page, '/')
  const form = page.getByRole('form', { name: 'new thread' })
  await pick(form, 'agent to start', { label: 'Fake agent' })
  if (preset) await pick(form, 'permissions', { label: preset })
  await form.getByRole('radio', { name: 'Worktree' }).click()
  await form.getByLabel('first prompt').fill(prompt)
  await form.getByLabel('first prompt').press('Enter')
  await page.waitForURL(/\/t\/[0-9a-f]+$/)
}

/** On a desktop, where the sidebar shows, waits for the thread list to say the thread waits on you. */
async function expectInNeedsYou(page: Page, title: string) {
  if ((page.viewportSize()?.width ?? 0) < 768) return
  await expect(page.getByRole('region', { name: 'needs you' })).toContainText(title)
}

test('a thread waiting on a permission', async ({ page }) => {
  await startFake(page, 'Add a hello line to main', 'Ask for everything')
  await shoot(page, 'thread-waiting', async (shown) => {
    await expect(shown.getByRole('group', { name: 'permission' })).toBeVisible()
    await expect(shown.getByTestId('agent-state')).toHaveText('Fake agent is waiting for your permission')
    // Grouped by turn, a running turn's work is one card, its step under way in the present tense (ADR 0049).
    await expect(shown.getByTestId('activity')).toContainText('Editing src/main.rs')
    await expectInNeedsYou(shown, 'Add a hello line to main')
  })
})

test('a plan to approve', async ({ page }) => {
  await startFake(page, 'Retry the upload twice', 'Plan')
  await shoot(page, 'thread-plan', async (shown) => {
    // The plan card follows the agent's state as it changes, ahead of the transcript, the preset and the
    // thread list, which are fetched once it does.
    await expect(shown.getByRole('group', { name: 'plan' })).toBeVisible()
    await expect(shown.getByText('Plan: Retry the upload twice')).toBeVisible()
    await expect(shown.getByLabel(/^after approving, the /)).toBeVisible()
    await expect(chip(shown, 'permissions')).toHaveAttribute('data-value', 'plan')
    await expectInNeedsYou(shown, 'Retry the upload twice')
  })
})

test('a question waiting on you', async ({ page }) => {
  await startFake(page, 'Ask me what to count')
  await shoot(page, 'thread-question', async (shown) => {
    await expect(shown.getByRole('region', { name: 'question' })).toContainText('waiting on you')
    await expect(shown.getByText('Waiting on you').filter({ visible: true }).first()).toBeVisible()
    await expectInNeedsYou(shown, 'Ask me what to count')
  })
})
