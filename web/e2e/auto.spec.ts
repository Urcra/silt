// Auto for an agent without an auto mode of its own (ADR 0129): a fake agent with no `auto` mode asks to run commands,
// another session of it judges them, and the thread shows each verdict, a refusal's reason, and the agent asking the
// person to allow what Auto refused. With SILT_SCREENSHOTS set to a folder, it keeps pictures of each for the pull
// request.
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
    data: { clone: join(fixture.work, 'repo'), agent: 'v1', prompt, preset: 'auto' },
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
    // Buttons ease into the scheme's colours; the picture waits for them to settle.
    await page.waitForTimeout(300)
    await page.screenshot({ path: join(folder, `${name}-${scheme}.png`) })
  }
  await page.emulateMedia({ colorScheme: null })
}

/** Opens the turn's work where it folded away once the turn ended, and each run of calls in it. */
async function unfold(page: Page) {
  // Once the turn has ended, its folds are drawn for good.
  await expect(page.getByTestId('agent-state').first()).toHaveText(/is ready/, { timeout: 30_000 })
  for (const folded of [
    'details[data-testid="worked"]:not([open]) > summary',
    'details[data-testid="tools"]:not([open]) > summary',
  ]) {
    // Each opened one leaves the match, so the first left is opened until none is.
    const left = page.locator(folded)
    for (let opened = 0; opened < 20 && (await left.count()) > 0; opened++) await left.first().click()
  }
}

test('a refused command says why, under its row, and the agent is told', async ({ page }) => {
  const fixture = start()
  await page.goto(fixture.url)
  const thread = await startThread(page, fixture, 'Run `rm -rf ~/.cache`')
  await page.goto(`${new URL(fixture.url).origin}/t/${thread}`)
  await expect(page.getByText('I was not allowed to run rm -rf ~/.cache.')).toBeVisible({ timeout: 30_000 })
  // Silt tells the agent why as a message of its own once its turn ends.
  await expect(page.getByText(/Silt's Auto check refused/).first()).toBeVisible({ timeout: 30_000 })
  await unfold(page)
  const refused = page.getByTestId('refused').first()
  await expect(refused).toContainText('deletes or rewrites what you didn')
  await expect(refused).toContainText('Sent back to the agent.')
  await expect(page.getByText('Auto refused').first()).toBeVisible()
  await refused.scrollIntoViewIfNeeded()
  await picture(page, '3-refused')
  await page.setViewportSize({ width: 390, height: 844 })
  await refused.scrollIntoViewIfNeeded()
  await picture(page, '7-refused-phone')
})

test('the agent asks the person to allow what Auto refused, and goes on once they do', async ({ page }) => {
  const fixture = start()
  await page.goto(fixture.url)
  const thread = await startThread(page, fixture, 'Run `git push --force origin fix` and escalate if refused')
  await page.goto(`${new URL(fixture.url).origin}/t/${thread}`)
  const card = page.getByRole('group', { name: 'escalation' })
  await expect(card).toBeVisible({ timeout: 30_000 })
  await expect(card).toContainText('Fake v1 asks you to allow what Auto refused')
  await expect(card).toContainText('git push --force origin fix')
  await expect(card).toContainText('The person asked for it.')
  await expect(page.getByTestId('agent-state')).toHaveText('Fake v1 is waiting for you to allow what Auto refused')
  await card.getByLabel(/Words for Fake v1/).fill("Fine, it's your branch.")
  await picture(page, '4-escalation')
  await page.setViewportSize({ width: 390, height: 844 })
  await card.scrollIntoViewIfNeeded()
  await picture(page, '8-escalation-phone')
  await page.setViewportSize({ width: 1440, height: 900 })
  await card.getByRole('button', { name: /Allow once/ }).click()
  await expect(page.getByText(/Ran it after all\./)).toBeVisible({ timeout: 30_000 })
  await unfold(page)
  await expect(page.getByText('allowed by you').first()).toBeVisible()
})

test('the permissions menu says who checks for Auto, and a thread picks another agent', async ({ page }) => {
  const fixture = start()
  await page.goto(fixture.url)
  const thread = await startThread(page, fixture, 'Run `cargo test -p upload`')
  await page.goto(`${new URL(fixture.url).origin}/t/${thread}`)
  await expect(page.getByText('Ran cargo test -p upload.')).toBeVisible({ timeout: 30_000 })
  await unfold(page)
  await expect(page.getByText('Auto allowed').first()).toBeVisible()

  await page.getByRole('button', { name: 'permissions', exact: true }).click()
  const menu = page.getByRole('menu', { name: 'permissions', exact: true })
  await expect(menu.locator('[data-id="auto"]')).toContainText('Silt checks each command first')
  await expect(menu.locator('[data-id="auto"]')).not.toHaveAttribute('aria-disabled', 'true')
  const row = menu.getByRole('menuitem', { name: 'who checks for Auto' })
  await expect(row).toContainText('Same as this thread')
  await picture(page, '1-menu')
  await row.click()
  const list = page.getByRole('menu', { name: 'who checks for Auto' })
  await expect(list.getByRole('menuitemradio', { name: /Same as this thread/ })).toHaveAttribute('aria-checked', 'true')
  await picture(page, '2-who-checks')
  await list.getByRole('menuitemradio', { name: /Fake other/ }).click()
  // The menu stays open, its row now naming the agent chosen.
  await expect(list).toBeHidden()
  await expect(row).toContainText('Fake other')
})

test('Settings › Auto chooses who checks in each workspace, from the agents it allows', async ({ page }) => {
  const fixture = start()
  await page.goto(fixture.url)
  await page.goto(`${new URL(fixture.url).origin}/settings/auto`)
  const section = page.getByRole('region', { name: 'Auto in Default' })
  await expect(section).toBeVisible({ timeout: 30_000 })
  await expect(section).toContainText('Any agent Default allows.')
  await expect(section).toContainText('Fake v1')
  await picture(page, '5-settings')
  await section.getByRole('button', { name: 'who checks for Auto in Default' }).click()
  await page.getByRole('menuitemradio', { name: 'Fake other' }).click()
  await expect(section.getByRole('button', { name: 'who checks for Auto in Default' })).toContainText('Fake other')
  await page.setViewportSize({ width: 390, height: 844 })
  await page.getByRole('heading', { name: 'Auto', exact: true }).scrollIntoViewIfNeeded()
  await picture(page, '6-settings-phone')
})
