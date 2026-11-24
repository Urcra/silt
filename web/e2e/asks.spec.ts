// Agents asking other agents (ADR 0120): one fake agent asks another, and the thread shows the ask as a card, the asked
// thread nested under it and saying who asked. With SILT_SCREENSHOTS set to a folder, it keeps pictures of each for
// the pull request.
import { execFileSync } from 'node:child_process'
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { type Page, expect, test } from '@playwright/test'
import { type Fixture, runFixture, SILT } from './fixture'

let world: Fixture | undefined

// Pictures for a pull request are taken as a person sees them, at twice the pixels.
if (process.env.SILT_SCREENSHOTS) test.use({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2 })

test.afterEach(() => {
  if (!world) return
  execFileSync(SILT, ['daemon', 'stop'], { env: { ...process.env, SILT_HOME: world.siltHome } })
  rmSync(world.work, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 })
  world = undefined
})

function start(): Fixture {
  world = JSON.parse(runFixture('fixture.sh', { ...process.env, SILT_ASK_AGENTS: '0' })) as Fixture
  const agent = join(import.meta.dirname, '..', '..', 'target', 'debug', 'silt-fake-agent')
  writeFileSync(
    join(world.siltHome, 'agents.json'),
    JSON.stringify([
      { id: 'v1', name: 'Fake v1', command: agent },
      { id: 'other', name: 'Fake other', command: agent },
    ]),
  )
  return world
}

async function startThread(page: Page, fixture: Fixture, prompt: string): Promise<string> {
  const token = new URL(fixture.url).hash.replace('#token=', '')
  const origin = new URL(fixture.url).origin
  const started = await page.request.post(`${origin}/api/agents/start`, {
    headers: { Authorization: `Bearer ${token}` },
    data: { clone: join(fixture.work, 'repo'), agent: 'v1', prompt },
  })
  expect(started.ok()).toBe(true)
  const answer = (await started.json()) as string | { thread: string }
  return typeof answer === 'string' ? answer : answer.thread
}

/** Keeps a picture of the page for the pull request, in light and dark, when asked to. */
async function picture(page: Page, name: string): Promise<void> {
  const folder = process.env.SILT_SCREENSHOTS
  if (!folder) return
  mkdirSync(folder, { recursive: true })
  await page.evaluate(() => document.fonts.ready)
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme })
    await page.screenshot({ path: join(folder, `${name}-${scheme}.png`) })
  }
  await page.emulateMedia({ colorScheme: null })
}

test("an agent asks another, and the ask's card, the asked thread and its report show", async ({ page }) => {
  const fixture = start()
  await page.goto(fixture.url)
  const thread = await startThread(page, fixture, 'Ask agent other: Review src/main.rs and report what you find.')
  const origin = new URL(fixture.url).origin
  await page.goto(`${origin}/t/${thread}`)

  // Once the turn ends, its card folds into what the agent did, as a subagent's does.
  await expect(page.getByText('Done: Ask agent other: Review src/main.rs')).toBeVisible({ timeout: 30_000 })
  const card = page.getByTestId('ask')
  if (!(await card.isVisible())) await page.locator('summary', { hasText: 'Worked' }).first().click()
  await expect(card).toBeVisible()
  await expect(card).toContainText('Fake other')
  await expect(card).toContainText('Review the change')
  await expect(card.getByText('done', { exact: true })).toBeVisible()
  await card.getByRole('button', { name: /Fake other/ }).click()
  await expect(card.getByText('Reported back')).toBeVisible()
  await expect(card.getByText('In your working tree')).toBeVisible()
  await expect(card.getByText('Review src/main.rs and report what you find.').first()).toBeVisible()
  await picture(page, '1-reported')
  await page.setViewportSize({ width: 390, height: 844 })
  await card.scrollIntoViewIfNeeded()
  await picture(page, '4-phone')
  await page.setViewportSize({ width: 1440, height: 900 })

  // The asked thread nests under its caller in the sidebar, and says who asked.
  const nav = page.getByRole('navigation', { name: 'projects' })
  await expect(nav.getByTestId('asked-mark')).toBeVisible()
  await card.getByRole('link', { name: 'Open its thread' }).click()
  await expect(page.getByTestId('asked-by')).toContainText('asked by')
  await expect(page.getByTestId('asked-note')).toContainText('Fake v1 asked for this thread')
  await page.getByTestId('asked-note').scrollIntoViewIfNeeded()
  await picture(page, '2-asked-thread')
})

test('the person stops an ask that runs past its caller, from the strip above the composer', async ({ page }) => {
  const fixture = start()
  await page.goto(fixture.url)
  const thread = await startThread(page, fixture, 'Ask agent other: Ponder long over the retry, no wait.')
  await page.goto(`${new URL(fixture.url).origin}/t/${thread}`)
  const strip = page.getByTestId('running-asks')
  await expect(strip).toBeVisible({ timeout: 30_000 })
  await expect(strip).toContainText('Fake other')
  await page.locator('summary', { hasText: 'Worked' }).first().click()
  await page
    .getByTestId('ask')
    .getByRole('button', { name: /Fake other/ })
    .click()
  await expect(page.getByTestId('ask').getByText('running', { exact: true })).toBeVisible()
  await picture(page, '3-running')
  await strip.getByRole('button', { name: 'Stop Fake other' }).click()
  await expect(strip).toBeHidden()
  // Its caller is told, which wakes it; the ask's card is then in the earlier turn's fold.
  await expect(page.getByText(/ended without a report: you stopped it/)).toBeVisible({ timeout: 15_000 })
  const card = page.getByTestId('ask')
  for (const fold of await page.locator('summary', { hasText: 'Worked' }).all()) {
    if (await card.isVisible()) break
    await fold.click()
  }
  await expect(card.getByText('stopped', { exact: true })).toBeVisible()
})

test("a workspace's rules say whether agents may ask other agents, and save", async ({ page }) => {
  const fixture = start()
  const url = new URL(fixture.url)
  await page.goto(fixture.url)
  await page.goto(`${url.origin}/settings/workspaces${url.hash}`)
  const rules = page.getByRole('region', { name: 'Rules' })
  await expect(rules).toContainText('Agents asking other agents')
  await expect(rules).toContainText('Allowed, as a command')
  await expect(rules).toContainText('No: one level')
  await rules.getByText('Agents asking other agents').scrollIntoViewIfNeeded()
  await picture(page, '5-rules')
  await rules.getByRole('radio', { name: 'Never' }).click()
  await expect(rules).toContainText('Never', { timeout: 10_000 })
  await expect(rules.getByRole('radio', { name: 'Never' })).toHaveAttribute('aria-checked', 'true')
})
