// Dictation in the composer, with Chromium's fake microphone and a stand-in
// for the speech helper.

import { execFileSync } from 'node:child_process'
import { rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { type Page, expect, test } from '@playwright/test'
import { type Fixture, runFixture, SILT } from './fixture'

test.use({
  launchOptions: {
    executablePath: process.env.SILT_E2E_CHROMIUM,
    args: ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream'],
  },
})

const HELPER = join(import.meta.dirname, '..', '..', 'target', 'debug', 'silt-fake-voice')

let started: Fixture | undefined

function start(env: Record<string, string> = {}, helper = HELPER): Fixture {
  // Asking the agents what they offer would fetch them through `npx` into the fixture's home, which no test here needs.
  started = JSON.parse(
    runFixture('fixture.sh', { ...process.env, SILT_VOICE_BIN: helper, SILT_ASK_AGENTS: '0', ...env }),
  ) as Fixture
  return started
}

test.afterEach(() => {
  if (!started) return
  execFileSync(SILT, ['daemon', 'stop'], { env: { ...process.env, SILT_HOME: started.siltHome } })
  // A daemon's agents can still be writing there for a moment after it stops.
  rmSync(started.work, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 })
  started = undefined
})

async function openThread(page: Page, world: Fixture): Promise<void> {
  await page.goto(world.url)
  await page.goto(`${new URL(world.url).origin}/t/${world.thread}`)
}

const dictate = (page: Page) => page.getByRole('button', { name: 'Dictate', exact: true })
const stopDictating = (page: Page) => page.getByRole('button', { name: 'Stop dictating' })
const draft = (page: Page) => page.getByLabel('draft')
const heard = (page: Page) => page.getByLabel('heard so far')
const bar = (page: Page) => page.getByRole('group', { name: 'dictation' })

/** Accepts the offer of the speech model, which comes only when dictation is first asked for. */
async function fetchModel(page: Page): Promise<void> {
  await dictate(page).click()
  await page.getByRole('dialog', { name: 'download the speech model' }).getByRole('button', { name: 'Download' }).click()
  await expect(page.getByRole('dialog', { name: 'download the speech model' })).toHaveCount(0)
  await expect(page.getByText(/Downloading the speech model/)).toHaveCount(0)
  await expect(dictate(page)).toBeEnabled()
}

test('no button without the speech helper', async ({ page }) => {
  await openThread(page, start({}, '/nonexistent'))
  await expect(draft(page)).toBeVisible()
  await expect(dictate(page)).toHaveCount(0)
})

test('the words show at the caret as they come, and join the draft there when the take ends', async ({ page }) => {
  await openThread(page, start())
  await fetchModel(page)
  await draft(page).fill('Please now')
  await draft(page).evaluate((box: HTMLTextAreaElement) => box.setSelectionRange(6, 6))
  await dictate(page).click()
  await expect(heard(page)).toHaveText(' hello there ')
  // Under the box, the take takes the settings' place.
  await expect(bar(page)).toBeVisible()
  await expect(page.getByLabel('agent to start')).toBeHidden()
  await expect(draft(page)).toHaveValue('Please now')
  await stopDictating(page).click()
  await expect(draft(page)).toHaveValue('Please Hello there. now')
  await expect(heard(page)).toHaveCount(0)
  await expect(bar(page)).toHaveCount(0)
  await expect(page.getByLabel('agent to start')).toBeVisible()
})

test('the model is not downloaded until the person agrees', async ({ page }) => {
  const world = start()
  await openThread(page, world)
  await dictate(page).click()
  await expect(page.getByText(/103 MB/)).toBeVisible()
  await page.getByRole('button', { name: 'Not now' }).click()
  await expect(page.getByText(/103 MB/)).toHaveCount(0)
  const status = execFileSync('ls', [world.siltHome], { encoding: 'utf8' })
  expect(status).not.toContain('models')
})

test('✓ keeps the words without sending them', async ({ page }) => {
  await openThread(page, start())
  await fetchModel(page)
  await dictate(page).click()
  await expect(heard(page)).toHaveText('hello there')
  await bar(page).getByRole('button', { name: 'Keep the words' }).click()
  await expect(draft(page)).toHaveValue('Hello there.')
})

test('⏎ during a take keeps the words and sends the draft', async ({ page }) => {
  await openThread(page, start())
  await fetchModel(page)
  await dictate(page).click()
  await expect(heard(page)).toHaveText('hello there')
  await page.keyboard.press('Enter')
  await expect(page.getByText('Hello there.').first()).toBeVisible()
  await expect(draft(page)).toHaveValue('')
  await expect(dictate(page)).toBeEnabled()
})

test('a pause leaves the take running', async ({ page }) => {
  await openThread(page, start({ FAKE_VOICE_PAUSE: '1' }))
  await fetchModel(page)
  await dictate(page).click()
  await expect(heard(page)).toHaveText('Hello there.')
  await expect(stopDictating(page)).toBeEnabled()
  await expect(draft(page)).toHaveValue('')
})

test('holding Ctrl+M talks until it is let go', async ({ page }) => {
  await openThread(page, start())
  await fetchModel(page)
  await draft(page).focus()
  await page.keyboard.down('Control')
  await page.keyboard.down('m')
  await expect(heard(page)).toHaveText('hello there')
  await expect(bar(page).getByText('release')).toBeVisible()
  await page.keyboard.up('m')
  await page.keyboard.up('Control')
  await expect(draft(page)).toHaveValue('Hello there.')
})

test('a tap of Ctrl+M starts a take, and another ends it', async ({ page }) => {
  await openThread(page, start())
  await fetchModel(page)
  await draft(page).focus()
  await page.keyboard.press('Control+m')
  await expect(heard(page)).toHaveText('hello there')
  await page.keyboard.press('Control+m')
  await expect(draft(page)).toHaveValue('Hello there.')
})

test('a take with no words neither fills the draft nor sends it', async ({ page }) => {
  await openThread(page, start({ FAKE_VOICE_SILENT: '1' }))
  await fetchModel(page)
  await dictate(page).click()
  await expect(stopDictating(page)).toBeVisible()
  await page.keyboard.press('Enter')
  // Which of the two depends on whether the fake microphone had beeped yet.
  await expect(page.getByText(/Silt didn't catch any words|Silt heard only silence/)).toBeVisible()
  await expect(draft(page)).toHaveValue('')
})

test('Escape throws the take away', async ({ page }) => {
  await openThread(page, start())
  await fetchModel(page)
  await draft(page).fill('Keep this')
  await dictate(page).click()
  await expect(heard(page)).toHaveText(' hello there')
  await page.keyboard.press('Escape')
  await expect(heard(page)).toHaveCount(0)
  await expect(dictate(page)).toBeEnabled()
  await expect(draft(page)).toHaveValue('Keep this')
  await expect(draft(page)).toBeFocused()
})

test('a take does not follow the person to another thread', async ({ page }) => {
  const world = start()
  await openThread(page, world)
  await fetchModel(page)
  await dictate(page).click()
  await expect(heard(page)).toHaveText('hello there')
  // Within the page, so the composer stays and only its thread changes.
  await page.evaluate((fork) => {
    window.history.pushState({}, '', `/t/${fork}`)
    window.dispatchEvent(new PopStateEvent('popstate'))
  }, world.fork)
  await expect(page).toHaveURL(new RegExp(`${world.fork}$`))
  await expect(heard(page)).toHaveCount(0)
  await expect(draft(page)).toHaveValue('')
})

test('a helper that dies says so and the button works again', async ({ page }) => {
  await openThread(page, start({ FAKE_VOICE_CRASH: '1' }))
  await fetchModel(page)
  await dictate(page).click()
  await expect(page.getByText('the speech helper stopped unexpectedly')).toBeVisible()
  await expect(dictate(page)).toBeEnabled()
})

test('the composer + menu opens upwards and stays in view', async ({ page }) => {
  await openThread(page, start())
  await page.getByRole('button', { name: 'add to the prompt' }).click()
  // The composer is docked at the bottom of a thread's page.
  await expect(page.getByRole('menuitem', { name: 'Attach files or images' })).toBeInViewport({ ratio: 1 })
})

test('a page hears the person through dictation once they agree, as the composer does (ADR 0079)', async ({ page }) => {
  const world = start()
  const agent = join(import.meta.dirname, '..', '..', 'target', 'debug', 'silt-fake-agent')
  writeFileSync(join(world.siltHome, 'agents.json'), JSON.stringify([{ id: 'v1', name: 'Fake v1', command: agent }]))
  const html =
    '<!doctype html><button id="talk">Talk</button><button id="stop">Stop</button><p id="out"></p><script>' +
    'let d=null;silt.use("dictation").then(function(x){d=x;if(d)d.onFinal(function(said){document.getElementById("out").textContent+=said.text})});' +
    'document.getElementById("talk").onclick=function(){if(d)d.start().catch(function(e){document.getElementById("out").textContent="refused: "+e.code})};' +
    'document.getElementById("stop").onclick=function(){if(d)d.stop()};</script>'
  const page_ = join(world.work, 'dictation-page.json')
  writeFileSync(page_, JSON.stringify({ title: 'Dictation page', kind: 'html', content: html, capabilities: { dictation: {} } }))
  const token = new URL(world.url).hash.replace('#token=', '')
  const origin = new URL(world.url).origin
  const started = await page.request.post(`${origin}/api/agents/start`, {
    headers: { Authorization: `Bearer ${token}` },
    data: { clone: join(world.work, 'repo'), agent: 'v1', prompt: `Artifact from ${page_}` },
  })
  expect(started.ok()).toBe(true)
  const answer = (await started.json()) as string | { thread: string }
  const thread = typeof answer === 'string' ? answer : answer.thread
  await page.goto(world.url)
  await page.goto(`${origin}/t/${thread}`)
  await expect(page.getByText(`Done: Artifact from ${page_}`)).toBeVisible({ timeout: 30_000 })
  // Opened beside the thread, or by its card where the panel didn't open by itself.
  const open = page.getByRole('button', { name: /^Open the artifact Dictation page/ })
  await expect(page.getByTestId('artifact-frame').or(open).first()).toBeVisible()
  if (!(await page.getByTestId('artifact-frame').isVisible())) await open.click()
  await expect(page.getByTestId('artifact-frame')).toBeVisible()
  const frame = page.frameLocator('[data-testid="artifact-frame"]')
  // The model is here once the person fetched it for the composer.
  await fetchModel(page)
  await frame.getByRole('button', { name: 'Talk' }).click()
  // The page asks first, in the page holding it.
  await page.getByRole('button', { name: 'Allow' }).click()
  await expect(page.getByText('The page is listening')).toBeVisible({ timeout: 15_000 })
  await frame.getByRole('button', { name: 'Stop' }).click()
  await expect(frame.locator('#out')).toContainText('Hello there.', { timeout: 15_000 })
})
