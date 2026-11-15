import { execFileSync } from 'node:child_process'
import { readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { type Page, expect, test } from '@playwright/test'
import { stopAgent } from './fixture'

const SILT = join(import.meta.dirname, '..', '..', 'target', 'debug', 'silt')

interface Live {
  alice: string
  bob: string
  /** Alice's second machine. */
  laptop: string
  server: string
  thread: string
  work: string
}

let live: Live

test.beforeAll(() => {
  const output = execFileSync(join(import.meta.dirname, 'live-fixture.sh'), { encoding: 'utf8' })
  live = JSON.parse(output.trim().split('\n').pop() ?? '{}') as Live
})

test.afterAll(() => {
  for (const home of ['alice', 'bob', 'alice-laptop']) {
    execFileSync(SILT, ['daemon', 'stop'], { env: { ...process.env, SILT_HOME: join(live.work, home, 'silt') } })
  }
  const pid = Number(readFileSync(join(live.work, 'server.pid'), 'utf8'))
  process.kill(pid)
  // A daemon's agents can still be writing there for a moment after it stops.
  rmSync(live.work, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 })
})

/** Opens All threads from the sidebar: `/` is a new thread (ADR 0044). */
async function allThreads(page: Page) {
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'All threads' }).click()
}

/** Opens a section of Settings, at the sidebar's bottom. */
async function settings(page: Page, section: string) {
  await page.getByRole('link', { name: 'Settings', exact: true }).click()
  await page.getByRole('navigation', { name: 'settings' }).getByRole('link', { name: section, exact: true }).click()
}

/** Picks the new thread's project from its picker. */
async function pickProject(page: Page, project: RegExp) {
  await page.getByRole('button', { name: /^project:/ }).click()
  await page.getByRole('dialog', { name: 'Pick a project' }).getByRole('option', { name: project }).click()
}

/** Starts the new thread in the background, from its start menu. */
async function startInTheBackground(page: Page) {
  await page.getByRole('button', { name: 'add to the prompt' }).click()
  await page.getByRole('menuitem', { name: /^Start in the background/ }).click()
}

/** Opens the thread's Live page. */
async function goLive(page: Page, url: string) {
  await page.goto(url)
  await allThreads(page)
  await page.getByRole('link', { name: live.thread }).click()
  await page.getByRole('button', { name: 'More for this thread' }).click()
  await page.getByRole('link', { name: /^Join live/ }).click()
}

async function openFile(page: Page, url: string) {
  await goLive(page, url)
  await page.getByRole('button', { name: /^src\/main\.rs/ }).click()
  await expect(page.getByTestId('editor')).toContainText('fn farewell() {}')
}

test('two people edit one thread live, see each other and their drafts', async ({ browser }) => {
  const alice = await (await browser.newContext()).newPage()
  const bob = await (await browser.newContext()).newPage()
  await openFile(alice, live.alice)
  await openFile(bob, live.bob)

  await expect(alice.getByTestId('presence')).toContainText('bob main.rs')
  await expect(bob.getByTestId('presence')).toContainText('alice main.rs')

  await alice.getByTestId('editor').locator('.cm-content').click()
  await alice.keyboard.press('ControlOrMeta+Home')
  await alice.keyboard.type('// alice was here\n')
  await expect(bob.getByTestId('editor')).toContainText('// alice was here', { timeout: 15_000 })

  await bob.getByTestId('editor').locator('.cm-content').click()
  await bob.keyboard.press('ControlOrMeta+End')
  await bob.keyboard.type('fn bob() {}\n')
  await expect(alice.getByTestId('editor')).toContainText('fn bob() {}', { timeout: 15_000 })

  await expect
    .poll(() => readFileSync(join(live.work, 'alice', 'repo', 'src', 'main.rs'), 'utf8'), { timeout: 15_000 })
    .toBe('// alice was here\nfn main() {}\nfn farewell() {}\nfn bob() {}\n')

  await bob.getByLabel('draft').fill('now add tests for bob')
  await expect(alice.getByRole('region', { name: 'next prompt' })).toContainText('bob is drafting')
  await expect(alice.getByRole('region', { name: 'next prompt' })).toContainText('now add tests for bob')
  await alice.context().close()
  await bob.context().close()
})

test('following a co-viewer shows their file and line', async ({ browser }) => {
  const alice = await (await browser.newContext()).newPage()
  const bob = await (await browser.newContext()).newPage()
  await goLive(alice, live.alice)
  await openFile(bob, live.bob)

  const code = bob.getByTestId('editor').locator('.cm-content')
  await code.click()
  await bob.keyboard.press('ControlOrMeta+Home')
  await bob.keyboard.press('ArrowDown')
  await bob.keyboard.press('ArrowDown')
  await expect(alice.getByTestId('presence')).toContainText('bob main.rs:3')

  await alice.getByRole('button', { name: 'Follow bob' }).click()
  await expect(alice.getByTestId('editor').locator('.cm-activeLine')).toHaveText('fn farewell() {}')
  await bob.keyboard.press('ArrowUp')
  await expect(alice.getByTestId('editor').locator('.cm-activeLine')).toHaveText('fn main() {}', { timeout: 10_000 })

  await alice.getByRole('button', { name: 'Stop following bob' }).click()
  await expect(alice.getByRole('button', { name: 'Follow bob' })).toBeVisible()
  await alice.context().close()
  await bob.context().close()
})

test('an agent started from the web UI takes everyone\'s drafts and asks before editing', async ({ browser }) => {
  const alice = await (await browser.newContext()).newPage()
  const bob = await (await browser.newContext()).newPage()
  // Alice runs the agent from the thread's page; Bob edits on its Live page.
  await alice.goto(live.alice)
  await allThreads(alice)
  await alice.getByRole('link', { name: live.thread }).click()
  await openFile(bob, live.bob)
  await bob.getByLabel('draft').fill('mind the tests')
  await expect(alice.getByTestId('drafts')).toContainText('bob is drafting: mind the tests')

  // The preset picked counts at once, even while the daemon is still taking it: starting the agent then starts it there.
  await alice.route('**/agent/preset', async (route) => {
    const taken = await route.fetch()
    await new Promise((resolve) => setTimeout(resolve, 1500))
    await route.fulfill({ response: taken })
  })
  await alice.getByLabel('permissions').selectOption('ask_for_everything')
  await alice.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await alice.getByLabel('draft').press('Enter')
  await expect(alice.getByTestId('agent-state')).toHaveText('Fake agent is ready')
  await alice.unrouteAll({ behavior: 'wait' })
  await expect(bob.getByTestId('agent-state')).toHaveText("alice's Fake agent is ready")

  // A prompt the daemon refuses comes back to the draft rather than stay shown as sent.
  await alice.route('**/agent/prompt', (route) => route.fulfill({ status: 400, json: { error: 'not now' } }))
  await alice.getByLabel('draft').fill('add acp')
  await alice.getByLabel('draft').press('Enter')
  await expect(alice.getByText('not now')).toBeVisible()
  await expect(alice.getByLabel('draft')).toHaveValue('add acp')
  await expect(alice.getByTestId('sending')).toHaveCount(0)
  // The prompt shows as sent at once, while the daemon takes its time.
  await alice.unroute('**/agent/prompt')
  await alice.route('**/agent/prompt', async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 2000))
    await route.fallback()
  })
  await alice.getByLabel('draft').press('Enter')
  await expect(alice.getByTestId('sending')).toHaveText('add acp', { timeout: 1000 })
  await expect(alice.getByLabel('draft')).toHaveValue('')
  await expect(alice.getByText('not now')).toHaveCount(0)
  await alice.unroute('**/agent/prompt')
  const asked = alice.getByRole('group', { name: 'permission' })
  // The prompt was held back 2 s above, and a loaded machine takes a while more to bring the agent's ask.
  await expect(asked).toContainText('Edit src/main.rs', { timeout: 15000 })
  await expect(alice.getByTestId('agent-state')).toHaveText('Fake agent is waiting for your permission')
  await expect(alice.getByRole('region', { name: 'needs you' })).toContainText('asks to run')
  await asked.getByRole('button', { name: 'Allow' }).click()

  await expect(alice.getByTestId('agent-state')).toHaveText('Fake agent is ready')
  // Replies are Markdown, a paragraph each, so the whole entry holds what the agent said.
  const reply = alice.getByTestId('entry').filter({ hasText: /^Done: You are continuing a conversation[\s\S]*add acp/ })
  await expect(reply).toContainText('add acp')
  await expect(reply).toContainText('bob: mind the tests')
  await expect(alice.getByText('add acp', { exact: true })).toBeVisible()
  await expect(bob.getByLabel('draft')).toHaveValue('')
  await expect(bob.getByTestId('editor')).toContainText('fn acp() {}', { timeout: 15_000 })

  await stopAgent(alice)
  await expect(alice.getByPlaceholder('Ask for the next step; sending starts the agent')).toBeVisible()
  await alice.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await alice.getByLabel('draft').press('Enter')
  await alice.getByLabel('draft').fill('once more')
  await alice.getByLabel('draft').press('Enter')
  await alice.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Allow' }).click()
  // A new process of this agent can't load its earlier session, so it starts
  // a new one primed with the conversation.
  await expect(alice.getByText(/^Done: You are continuing a conversation[\s\S]*once more$/)).toBeVisible()
  await alice.getByLabel('draft').fill('not this')
  await alice.getByLabel('draft').press('Enter')
  await alice.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Reject' }).click()
  await expect(alice.getByText('I was not allowed to edit.')).toBeVisible()
  // A question waits in the transcript where it was asked, answered by a choice.
  await alice.getByLabel('draft').fill('Ask me what to count')
  await alice.getByLabel('draft').press('Enter')
  const question = alice.getByRole('region', { name: 'question' }).last()
  await expect(question).toContainText('What should the progress bar count?')
  await expect(alice.getByTestId('agent-state')).toHaveText('Fake agent is asking you a question')
  await expect(alice.getByRole('region', { name: 'needs you' })).toContainText('asks you')
  await question.getByRole('radio', { name: /Bytes sent on the socket/ }).click()
  await expect(alice.getByText('Counting sent.')).toBeVisible()
  await expect(question).toContainText('answered')
  await expect(question).toContainText('Bytes sent on the socket')
  await question.getByRole('button', { name: 'Show the choices' }).click()
  await expect(question.getByRole('listitem').filter({ hasText: 'Bytes sent on the socket' })).toHaveClass(/bg-accent-tint/)
  // Or answered in your own words, which the agent hears as a steer.
  await alice.getByLabel('draft').fill('Ask me again')
  await alice.getByLabel('draft').press('Enter')
  const again = alice.getByRole('region', { name: 'question' }).last()
  await expect(again).toContainText('waiting on you')
  await again.getByLabel('something else').fill('Packets acknowledged')
  await again.getByRole('button', { name: 'Answer' }).click()
  await expect(again).toContainText('answered')
  await expect(again).toContainText('Packets acknowledged')
  await alice.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Reject' }).click()
  await expect(alice.getByText(/I was not allowed to edit/).last()).toBeVisible()
  // Claude's form follows each question with an optional box of your own and requires nothing: answered once something is chosen.
  await alice.getByLabel('draft').fill('Ask two things')
  await alice.getByLabel('draft').press('Enter')
  const two = alice.getByRole('region', { name: 'question' }).last()
  await expect(two).toContainText('2 of 2')
  const answer = two.getByRole('button', { name: 'Answer', exact: true })
  await expect(answer).toBeDisabled()
  await two.getByRole('radio', { name: 'Chunks' }).click()
  await two.getByRole('radio', { name: 'Footer' }).click()
  await two.getByLabel('Other').first().fill('every 64 KiB')
  await answer.click()
  await expect(alice.getByText('Counting Chunks in the Footer; notes: 1.')).toBeVisible()
  await expect(two).toContainText('Chunks, every 64 KiB')

  const changed = () =>
    alice.evaluate(async (thread) => {
      const get = async (path: string) =>
        (
          await fetch(`/api/threads/${thread}/${path}`, {
            headers: { Authorization: `Bearer ${sessionStorage.getItem('silt-token')}` },
          })
        ).json()
      const turns = (await get('turns')) as { number: number; prompt: string }[]
      const turn = turns.find((candidate) => candidate.prompt.startsWith('once more'))
      return turn ? ((await get(`diff?turn=${turn.number}`)) as string) : 'no such turn'
    }, live.thread)
  // The agent's reply can come before the daemon records its edit.
  await expect.poll(changed, { timeout: 15_000 }).toContain('+fn acp() {}')
  await alice.context().close()
  await bob.context().close()
})

async function startInNewThread(page: Page, agent: string) {
  await page.goto(live.alice)
  // A checkout of its own, so the tests' agents leave the clone as it is.
  await page.getByRole('radio', { name: 'Worktree' }).click()
  await page.getByLabel('agent to start').selectOption({ label: agent })
  await page.getByLabel('first prompt').press('Enter')
  await expect(page.getByTestId('agent-state')).toHaveText(`${agent} is ready`)
}

test('the server-hosted app shows an agent waiting on the machine running it', async ({ browser }) => {
  const machine = await (await browser.newContext()).newPage()
  await startInNewThread(machine, 'Fake agent')
  await machine.getByLabel('permissions').selectOption('ask_for_everything')
  await machine.getByLabel('draft').fill('wait for me')
  await machine.getByLabel('draft').press('Enter')
  const asked = machine.getByRole('group', { name: 'permission' })
  await expect(asked).toContainText('Edit src/main.rs')

  const hosted = await (await browser.newContext()).newPage()
  await hosted.goto(`${live.server}/app/`)
  await hosted.getByLabel('User').fill('alice')
  await hosted.getByLabel('Password').fill('correct horse battery')
  await hosted.getByRole('button', { name: 'Sign in' }).click()
  const waiting = hosted.getByRole('region', { name: 'needs you' }).getByRole('group', { name: /wait for me needs you/ })
  await expect(async () => {
    await hosted.reload()
    await expect(waiting).toBeVisible({ timeout: 1_000 })
  }).toPass({ timeout: 15_000 })
  await expect(waiting).toContainText('asks to run')

  // The browser has no daemon: what it asks goes through the server to the machine.
  await waiting.getByRole('link').first().click()
  const there = hosted.getByRole('main')
  await expect(there.getByTestId('agent-state')).toHaveText('Fake agent is waiting for your permission')
  await there.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Allow' }).click()
  await expect(machine.getByTestId('agent-state')).toHaveText('Fake agent is ready')
  await expect(machine.getByText(/^Done: wait for me/)).toBeVisible()
  await expect(there.getByTestId('agent-state')).toHaveText('Fake agent is ready')

  await there.getByLabel('draft').fill('from the browser')
  await there.getByLabel('draft').press('Enter')
  await expect(asked).toContainText('Edit src/main.rs')
  await expect(there.getByRole('group', { name: 'permission' })).toBeVisible()
  await there.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Reject' }).click()
  await expect(machine.getByText('I was not allowed to edit.')).toBeVisible()
  // A question the agent asks there is answered here, in place, through the server.
  await there.getByLabel('draft').fill('Ask me from the browser')
  await there.getByLabel('draft').press('Enter')
  const question = there.getByRole('region', { name: 'question' }).last()
  await expect(question).toContainText('waiting on you', { timeout: 15_000 })
  await question.getByRole('radio', { name: /Chunks completed/ }).click()
  await expect(machine.getByText('Counting chunks.')).toBeVisible()
  await expect(async () => {
    await hosted.goto(`${live.server}/app/`)
    await expect(waiting).toHaveCount(0, { timeout: 1_000 })
  }).toPass({ timeout: 15_000 })
  await stopAgent(machine)
  await machine.context().close()
  await hosted.context().close()
})

test('the server-hosted app continues a thread on a chosen device', async ({ browser }) => {
  const machine = await (await browser.newContext()).newPage()
  await startInNewThread(machine, 'Fake agent')
  await machine.getByLabel('draft').fill('first turn')
  await machine.getByLabel('draft').press('Enter')
  await expect(machine.getByText('Done: first turn')).toBeVisible()
  await stopAgent(machine)
  await expect(machine.getByPlaceholder('Ask for the next step; sending starts the agent')).toBeVisible()
  const thread = new URL(machine.url()).pathname.split('/t/')[1]

  const hosted = await (await browser.newContext()).newPage()
  await hosted.goto(`${live.server}/app/t/${thread}`)
  await hosted.getByLabel('User').fill('alice')
  await hosted.getByLabel('Password').fill('correct horse battery')
  await hosted.getByRole('button', { name: 'Sign in' }).click()
  const there = hosted.getByRole('main')
  await there.getByLabel('device to continue on').selectOption({ label: 'alice' })
  await there.getByLabel('agent to continue with').selectOption({ label: 'Fake agent' })
  await there.getByRole('button', { name: 'Continue there' }).click()
  await expect(machine.getByTestId('agent-state')).toHaveText('Fake agent is ready')
  await expect(there.getByTestId('agent-state')).toHaveText('Fake agent is ready')

  await there.getByLabel('draft').fill('second turn')
  await there.getByLabel('draft').press('Enter')
  await expect(machine.getByText(/^Done: [\s\S]*second turn$/)).toBeVisible()
  await stopAgent(there)
  await expect(machine.getByPlaceholder('Ask for the next step; sending starts the agent')).toBeVisible()
  await machine.context().close()
  await hosted.context().close()
})

test("another of Alice's machines directs the agent running on the first, through the server", async ({ browser }) => {
  // Its waits for the second machine to hear the agent through the server take up to 20 s of a test's 30 under load.
  test.slow()
  const machine = await (await browser.newContext()).newPage()
  await startInNewThread(machine, 'Fake agent')
  await machine.getByLabel('permissions').selectOption('ask_for_everything')
  await machine.getByLabel('draft').fill('from the desk')
  await machine.getByLabel('draft').press('Enter')
  const asked = machine.getByRole('group', { name: 'permission' })
  await expect(asked).toContainText('Edit src/main.rs')
  const thread = new URL(machine.url()).pathname.split('/t/')[1]

  const laptop = await (await browser.newContext()).newPage()
  await laptop.goto(live.laptop)
  const state = laptop.getByTestId('agent-state')
  await expect(async () => {
    await laptop.goto(new URL(`/t/${thread}`, live.laptop).toString())
    await expect(state).toHaveText('Fake agent on alice is waiting for your permission', { timeout: 2_000 })
  }).toPass({ timeout: 20_000 })
  await laptop.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Allow' }).click()
  await expect(machine.getByText(/^Done: [\s\S]*from the desk$/)).toBeVisible()
  await expect(state).toHaveText('Fake agent on alice is ready', { timeout: 10_000 })

  await laptop.getByLabel('draft').fill('from the laptop')
  await laptop.getByLabel('draft').press('Enter')
  await expect(asked).toContainText('Edit src/main.rs')
  await expect(laptop.getByRole('group', { name: 'permission' })).toBeVisible({ timeout: 10_000 })
  await laptop.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Reject' }).click()
  await expect(machine.getByText('I was not allowed to edit.').last()).toBeVisible()
  await stopAgent(laptop)
  await expect(machine.getByPlaceholder('Ask for the next step; sending starts the agent')).toBeVisible()
  await machine.context().close()
  await laptop.context().close()
})

test('a new thread starts a Claude Code cloud session through claude, and lists it until its thread arrives', async ({ page }) => {
  await page.goto(live.alice)
  await page.getByRole('radio', { name: 'Cloud' }).click()
  await expect(page.getByLabel('agent to start')).toBeHidden()
  await page.getByLabel('first prompt').fill('tidy the readme in the cloud')
  await page.getByLabel('first prompt').press('Enter')
  const launch = page.getByRole('region', { name: 'Cloud sessions' }).getByRole('listitem').filter({ hasText: 'tidy the readme in the cloud' })
  await expect(launch).toContainText('waiting for its thread', { timeout: 20_000 })
  await expect(launch.getByRole('link', { name: 'claude.ai' })).toHaveAttribute('href', 'https://claude.ai/code/session_01FakeCloud')
  await expect(page.getByLabel('first prompt')).toHaveValue('')
})

test('the inbox hears that an agent waits, and answers it in place', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('permissions').selectOption('ask_for_everything')
  await page.getByLabel('draft').fill('ask me in the inbox')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByRole('group', { name: 'permission' })).toContainText('Edit src/main.rs')
  const thread = new URL(page.url()).pathname.split('/t/')[1]

  const item = page
    .getByTestId('inbox')
    .getByRole('listitem')
    .filter({ hasText: thread.slice(-12) })
    .filter({ hasText: 'asks to run: Edit src/main.rs' })
  await expect(async () => {
    await page.goto(new URL('/inbox', live.alice).toString())
    await expect(item).toBeVisible({ timeout: 2_000 })
  }).toPass({ timeout: 20_000 })
  await item.getByRole('button', { name: 'Allow' }).click()
  await expect(item).toHaveCount(0)
  await page.goto(new URL(`/t/${thread}`, live.alice).toString())
  await expect(page.getByText(/^Done: [\s\S]*ask me in the inbox$/)).toBeVisible()
  await stopAgent(page)
})

test("a running turn's thought under way shows in its card, where it will be recorded", async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('draft').fill('Ponder the edit')
  await page.getByLabel('draft').press('Enter')
  const activity = page.getByTestId('activity')
  await expect(activity).toContainText('Checking what the edit left.')
  // Only there: not in the tail under it as well, to jump into the card once the next step records it.
  await expect(page.getByText('Checking what the edit left.')).toHaveCount(1)
  await expect(page.getByText(/^Done: [\s\S]*Ponder the edit$/)).toBeVisible()
  await stopAgent(page)
})

test('a waiting agent notifies its person in a tab that is open but not in front', async ({ browser }) => {
  const context = await browser.newContext()
  await context.addInitScript(() => {
    const shown: string[] = []
    Object.assign(window, { shown })
    class Recorder {
      static permission = 'granted'
      static requestPermission = async () => 'granted'
      onclick: (() => void) | null = null
      constructor(title: string, options?: NotificationOptions) {
        shown.push(`${title}: ${options?.body ?? ''}`)
      }
      close() {}
    }
    Object.assign(window, { Notification: Recorder })
    document.hasFocus = () => false
  })
  const watcher = await context.newPage()
  await watcher.goto(live.alice)

  const machine = await (await browser.newContext()).newPage()
  await startInNewThread(machine, 'Fake agent')
  await machine.getByLabel('permissions').selectOption('ask_for_everything')
  await machine.getByLabel('draft').fill('notify me')
  await machine.getByLabel('draft').press('Enter')
  const asked = machine.getByRole('group', { name: 'permission' })
  await expect(asked).toContainText('Edit src/main.rs')
  await expect
    .poll(() => watcher.evaluate(() => (window as unknown as { shown: string[] }).shown.join('\n')), { timeout: 40_000 })
    .toContain('An agent asks for a permission: Fake agent on alice asks to run: Edit src/main.rs')
  await asked.getByRole('button', { name: 'Allow' }).click()
  await stopAgent(machine)
  await context.close()
  await machine.context().close()
})

test('on a phone, the hosted app answers a waiting agent from Needs you', async ({ browser }) => {
  const machine = await (await browser.newContext()).newPage()
  await startInNewThread(machine, 'Fake agent')
  await machine.getByLabel('permissions').selectOption('ask_for_everything')
  await machine.getByLabel('draft').fill('from my phone')
  await machine.getByLabel('draft').press('Enter')
  await expect(machine.getByRole('group', { name: 'permission' })).toContainText('Edit src/main.rs')

  const phone = await (await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true })).newPage()
  await phone.goto(`${live.server}/app/needs-you`)
  await phone.getByLabel('User').fill('alice')
  await phone.getByLabel('Password').fill('correct horse battery')
  await phone.getByRole('button', { name: 'Sign in' }).click()
  await expect(phone).toHaveURL(`${live.server}/app/needs-you`)
  await expect(phone.getByRole('navigation', { name: 'tabs' })).toBeVisible()
  await expect(phone.getByRole('banner').getByRole('link', { name: 'Inbox' })).toBeHidden()
  const manifest = (await (await phone.request.get(`${live.server}/app/manifest.webmanifest`)).json()) as { name: string }
  expect(manifest.name).toBe('Silt')

  const waiting = phone.getByRole('region', { name: 'waiting on you' }).getByRole('group', { name: /from my phone needs you/ })
  await expect(async () => {
    await phone.reload()
    await expect(waiting.getByRole('button', { name: 'Allow' })).toBeVisible({ timeout: 2_000 })
  }).toPass({ timeout: 20_000 })
  await waiting.getByRole('button', { name: 'Allow' }).tap()
  await expect(machine.getByText(/^Done: [\s\S]*from my phone$/)).toBeVisible()
  await stopAgent(machine)
  await machine.context().close()
  await phone.context().close()
})

test('a phone paired by QR code answers a permission, but may not prompt until allowed', async ({ browser }) => {
  const desk = await (await browser.newContext()).newPage()
  await desk.goto(`${live.server}/app/devices`)
  await desk.getByLabel('User').fill('alice')
  await desk.getByLabel('Password').fill('correct horse battery')
  await desk.getByRole('button', { name: 'Sign in' }).click()
  await desk.getByRole('region', { name: 'pair a phone' }).getByRole('button', { name: 'Show a code' }).click()
  await expect(desk.getByRole('img', { name: 'QR code to pair a phone' })).toBeVisible()
  const code = (await desk.getByTestId('pairing-code').textContent()) ?? ''

  const phone = await (await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true })).newPage()
  await phone.goto(`${live.server}/pair`)
  await phone.getByLabel('Code').fill(code.toLowerCase())
  await phone.getByRole('button', { name: 'Continue' }).click()
  await expect(phone.getByText('Sign this phone in to Silt as alice?')).toBeVisible()
  await phone.getByRole('button', { name: 'Sign in' }).click()
  await expect(phone).toHaveURL(`${live.server}/app/needs-you`)
  await desk.getByRole('button', { name: "It's paired" }).click()
  await expect(desk.getByRole('region', { name: 'Phone', exact: true })).toContainText('phone')

  const machine = await (await browser.newContext()).newPage()
  await startInNewThread(machine, 'Fake agent')
  await machine.getByLabel('permissions').selectOption('ask_for_everything')
  await machine.getByLabel('draft').fill('paired phone')
  await machine.getByLabel('draft').press('Enter')
  await expect(machine.getByRole('group', { name: 'permission' })).toContainText('Edit src/main.rs')
  const thread = new URL(machine.url()).pathname.split('/t/')[1]

  const waiting = phone.getByRole('region', { name: 'waiting on you' }).getByRole('group', { name: /paired phone needs you/ })
  await expect(async () => {
    await phone.reload()
    await expect(waiting.getByRole('button', { name: 'Allow' })).toBeVisible({ timeout: 2_000 })
  }).toPass({ timeout: 20_000 })
  await waiting.getByRole('button', { name: 'Allow' }).tap()
  await expect(machine.getByText(/^Done: [\s\S]*paired phone$/)).toBeVisible()

  await phone.goto(`${live.server}/app/t/${thread}`)
  const there = phone.getByRole('main')
  await expect(there.getByTestId('agent-state')).toHaveText('Fake agent is ready', { timeout: 15_000 })
  await there.getByLabel('draft').fill('may I?')
  await there.getByRole('button', { name: 'Send' }).tap()
  await expect(there.getByText('this device may not send prompts; change that on the Devices page')).toBeVisible()
  await stopAgent(machine)
  await desk.context().close()
  await phone.context().close()
  await machine.context().close()
})

test('the server-hosted app reviews a merge request through a machine, and ticks its files seen', async ({ browser }) => {
  const machine = await (await browser.newContext()).newPage()
  await startInNewThread(machine, 'Fake agent')
  await stopAgent(machine)
  await machine.getByRole('button', { name: 'Fork', exact: true }).click()
  await expect(machine.getByRole('main').getByText(/^Fork of/)).toBeVisible()
  await machine.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await machine.getByLabel('draft').press('Enter')
  await machine.getByLabel('draft').fill('fork work')
  await machine.getByLabel('draft').press('Enter')
  await expect(machine.getByText(/^Done: [\s\S]*fork work$/)).toBeVisible()
  await stopAgent(machine)
  // The header's; the changes bar over the composer offers it too.
  await machine.getByRole('button', { name: 'Offer as merge request' }).first().click()
  await machine.getByLabel('merge request title').fill('Fork work')
  await machine.getByRole('button', { name: 'Offer', exact: true }).click()
  await expect(machine).toHaveURL(/\/mr\//)
  const mr = new URL(machine.url()).pathname.split('/mr/')[1]

  const hosted = await (await browser.newContext()).newPage()
  await hosted.goto(`${live.server}/app/mr/${mr}`)
  await hosted.getByLabel('User').fill('alice')
  await hosted.getByLabel('Password').fill('correct horse battery')
  await hosted.getByRole('button', { name: 'Sign in' }).click()
  await expect(hosted.getByTestId('mr-status')).toHaveText('open')
  const seen = hosted.getByLabel('seen src/main.rs')
  await expect(async () => {
    await hosted.reload()
    await expect(seen).toBeVisible({ timeout: 2_000 })
  }).toPass({ timeout: 20_000 })
  await expect(hosted.getByText('+ fn acp() {}').first()).toBeVisible()
  await expect(hosted.getByTestId('seen-count')).toHaveText('0/1 seen')
  const marked = hosted.waitForResponse((response) => response.url().endsWith('/seen'))
  await seen.click()
  await expect(hosted.getByTestId('seen-count')).toHaveText('1/1 seen')
  expect((await marked).status()).toBe(200)
  await hosted.reload()
  await expect(hosted.getByTestId('seen-count')).toHaveText('1/1 seen')
  // The machine hears of it too, as it would from another of its person's devices.
  await machine.reload()
  await expect(machine.getByTestId('seen-count')).toHaveText('1/1 seen')
  // Declined, so the thread doesn't wait on Alice's review in later tests.
  await hosted.getByRole('button', { name: 'Decline' }).click()
  await expect(hosted.getByTestId('mr-status')).toHaveText('declined')
  await machine.context().close()
  await hosted.context().close()
})

test('a fork from an earlier turn has the conversation up to it', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('draft').fill('the first turn')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText(/^Done: [\s\S]*the first turn$/)).toBeVisible()
  await page.getByLabel('draft').fill('the second turn')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText(/^Done: [\s\S]*the second turn$/)).toBeVisible()
  await stopAgent(page)
  const source = new URL(page.url()).pathname

  await page.getByRole('button', { name: 'fork from turn 1' }).click()
  await expect(page.getByRole('main').getByText(/^Fork of/)).toBeVisible()
  expect(new URL(page.url()).pathname).not.toBe(source)
  const history = page.getByTestId('history')
  await expect(history).toContainText('the first turn')
  await expect(history).not.toContainText('the second turn')
})

test('a revert takes the files back to a turn and keeps the later turns, rewound', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  for (const prompt of ['revert keeps this', 'revert drops this']) {
    await page.getByLabel('draft').fill(prompt)
    await page.getByLabel('draft').press('Enter')
    await expect(page.getByText(new RegExp(`^Done: [\\s\\S]*${prompt}$`))).toBeVisible()
  }
  await stopAgent(page)
  await expect(page.getByPlaceholder('Ask for the next step; sending starts the agent')).toBeVisible()
  const thread = new URL(page.url()).pathname.split('/').pop() ?? ''
  const edits = () =>
    page.evaluate(async (thread) => {
      const response = await fetch(`/api/threads/${thread}/diff`, {
        headers: { Authorization: `Bearer ${sessionStorage.getItem('silt-token')}` },
      })
      return ((await response.json()) as string).split('+fn acp() {}').length - 1
    }, thread)
  // The agent's reply can come before the daemon records its edit.
  await expect.poll(edits, { timeout: 15_000 }).toBe(2)

  await page.getByRole('button', { name: 'revert to turn 1' }).click()
  await page.getByRole('button', { name: 'Revert', exact: true }).click()
  await expect(page.getByText(/^Reverted to turn 1/)).toBeVisible()
  await expect.poll(edits, { timeout: 15_000 }).toBe(1)
  const history = page.getByTestId('history')
  await expect(history.getByTestId('entry').filter({ has: page.getByText('revert drops this', { exact: true }) })).toHaveAttribute('data-rewound', 'true')
  await expect(history.getByTestId('entry').filter({ has: page.getByText('revert keeps this', { exact: true }) })).not.toHaveAttribute('data-rewound')

  // The agent started next is primed without the rewound turn.
  await page.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByTestId('agent-state')).toHaveText('Fake agent is ready')
  await page.getByLabel('draft').fill('after the revert')
  await page.getByLabel('draft').press('Enter')
  const reply = page.getByText(/^Done: You are continuing a conversation[\s\S]*after the revert$/)
  await expect(reply).toContainText('revert keeps this')
  await expect(reply).not.toContainText('revert drops this')
  await stopAgent(page)
})

test('permalinks open a turn, a comment and lines of a file, here and in the server-hosted app', async ({ browser, page }) => {
  await startInNewThread(page, 'Fake agent')
  for (const prompt of ['permalink first', 'permalink second']) {
    await page.getByLabel('draft').fill(prompt)
    await page.getByLabel('draft').press('Enter')
    await expect(page.getByText(new RegExp(`^Done: [\\s\\S]*${prompt}$`))).toBeVisible()
  }
  await stopAgent(page)
  const thread = new URL(page.url()).pathname.split('/').pop() ?? ''
  const copied = async (button: ReturnType<Page['getByRole']>) => {
    await button.click()
    const text = (await page.getByText(/^Link copied: /).last().textContent()) ?? ''
    return text.replace(/^Link copied: /, '')
  }
  // This machine is on a server, so links point into the server's app.
  const here = (link: string) => new URL(`/t/${thread}${new URL(link).hash}`, live.alice).toString()
  const entry = (on: Page, text: string) => on.getByTestId('entry').filter({ has: on.getByText(text, { exact: true }) })

  const turn = await copied(page.getByRole('button', { name: 'link to turn 1' }))
  expect(turn).toMatch(/\/app\/t\/[0-9a-f]+#turn-1$/)
  await page.goto(live.alice)
  await page.goto(here(turn))
  await expect(entry(page, 'permalink first')).toHaveAttribute('data-highlighted', 'true')
  await expect(entry(page, 'permalink second')).not.toHaveAttribute('data-highlighted')

  await entry(page, 'permalink second').getByText('permalink second', { exact: true }).hover()
  await entry(page, 'permalink second').getByRole('button', { name: /^comment on entry \d+$/ }).click()
  await page.getByLabel('comment', { exact: true }).fill('link to this')
  await page.getByRole('button', { name: 'Comment', exact: true }).click()
  const said = page.getByRole('group', { name: 'comment by alice' }).filter({ hasText: 'link to this' })
  const comment = await copied(said.getByRole('button', { name: 'link', exact: true }))
  expect(comment).toMatch(/#comment-/)
  await page.goto(live.alice)
  await page.goto(here(comment))
  await expect(said).toHaveAttribute('data-highlighted', 'true')

  await page.getByRole('region', { name: 'changes' }).getByRole('link', { name: 'view file' }).first().click()
  const file = page.getByTestId('file')
  await expect(file).toContainText('fn acp() {}')
  await page.getByRole('button', { name: 'line 1', exact: true }).click()
  await page.getByRole('button', { name: 'line 2', exact: true }).click({ modifiers: ['Shift'] })
  const lines = await copied(page.getByRole('button', { name: 'copy link to these lines' }))
  expect(lines).toMatch(/\/app\/t\/[0-9a-f]+\/files\/src\/main\.rs\?at=[^#]+#L1-L2$/)

  const hosted = await (await browser.newContext()).newPage()
  await hosted.goto(turn)
  await hosted.getByLabel('User').fill('alice')
  await hosted.getByLabel('Password').fill('correct horse battery')
  await hosted.getByRole('button', { name: 'Sign in' }).click()
  await expect(async () => {
    await hosted.goto(turn)
    await expect(entry(hosted, 'permalink first')).toHaveAttribute('data-highlighted', 'true', { timeout: 2_000 })
  }).toPass({ timeout: 20_000 })
  // The server asks one of Alice's machines for the file, as it lacks the git text it starts from.
  await hosted.goto(lines)
  await expect(hosted.getByTestId('file')).toContainText('fn acp() {}')
  await expect(hosted.locator('[data-picked]')).toHaveCount(2)
  await hosted.context().close()
})

test('a read-only link opens the conversation without signing in, until revoked', async ({ browser }) => {
  const owner = await (await browser.newContext()).newPage()
  await owner.goto(live.alice)
  await owner.goto(new URL(`/t/${live.thread}`, live.alice).toString())
  await owner.getByRole('button', { name: 'Share' }).click()
  const links = owner.getByRole('region', { name: 'read-only links' })
  await links.getByLabel('link lasts').selectOption('1')
  await links.getByRole('button', { name: 'Make a link' }).click()
  const url = await links.getByTestId('new-link').inputValue()
  expect(url).toContain('/r/')

  const anyone = await (await browser.newContext()).newPage()
  await anyone.goto(url)
  await expect(anyone.getByText('Please add a farewell line to main').first()).toBeVisible()
  await expect(anyone.getByText('Turn 1')).toBeVisible()

  await links.getByRole('button', { name: 'Revoke' }).first().click()
  await expect(async () => {
    await anyone.reload()
    await expect(anyone.getByRole('heading', { name: 'Not available' })).toBeVisible({ timeout: 1_000 })
  }).toPass({ timeout: 10_000 })
  await owner.context().close()
  await anyone.context().close()
})

test('a comment written in the server-hosted app reaches the machines', async ({ browser }) => {
  const hosted = await (await browser.newContext()).newPage()
  await hosted.goto(`${live.server}/app/t/${live.thread}`)
  await hosted.getByLabel('User').fill('bob')
  await hosted.getByLabel('Password').fill('correct horse battery')
  await hosted.getByRole('button', { name: 'Sign in' }).click()
  await hosted.getByTestId('history').getByText('Please add a farewell line to main', { exact: true }).first().hover()
  await hosted.getByRole('button', { name: 'comment on entry 0' }).click()
  await hosted.getByLabel('comment', { exact: true }).fill('said in the browser')
  await hosted.getByRole('button', { name: 'Comment', exact: true }).click()
  await expect(hosted.getByText(/: said in the browser/)).toBeVisible()

  const machine = await (await browser.newContext()).newPage()
  await machine.goto(live.alice)
  await expect(async () => {
    await machine.goto(new URL(`/t/${live.thread}`, live.alice).toString())
    await expect(machine.getByText(/bob: said in the browser/)).toBeVisible({ timeout: 2_000 })
  }).toPass({ timeout: 20_000 })
  await hosted.context().close()
  await machine.context().close()
})

test('the Devices page shows which device this is, that it is online, and what it may do', async ({ page }) => {
  await page.goto(live.alice)
  await settings(page, 'Devices and sync')
  const here = page.getByRole('region').filter({ has: page.getByText('this device', { exact: true }) })
  await expect(here.getByText('online', { exact: true })).toBeVisible()
  await expect(page.getByRole('region', { name: 'alice-laptop' }).getByText('online', { exact: true })).toBeVisible()
  await here.getByText('What it may do on your other devices').click()
  const prompts = here.getByLabel('Send prompts')
  await expect(prompts).toBeChecked()
  await prompts.click()
  await expect(prompts).not.toBeChecked()
  await page.reload()
  await here.getByText('What it may do on your other devices').click()
  await expect(prompts).not.toBeChecked()
  await prompts.click()
  await expect(prompts).toBeChecked()
})

test('a new thread starts with an agent in a known clone, which edits without asking by default', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await expect(page.getByLabel('permissions')).toHaveValue('ask_before_commands')
  await page.getByLabel('draft').fill('hello')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: hello')).toBeVisible()
  await expect(page.getByRole('group', { name: 'permission' })).toHaveCount(0)
  // What it changed sits over the composer; with nothing committed on a branch, Create PR is off and says why.
  const changed = page.getByRole('group', { name: 'what the thread changed' })
  await expect(changed).toContainText('1 file changed')
  const createPr = changed.getByRole('button', { name: 'Create PR' })
  await expect(createPr).toBeDisabled()
  await expect(createPr).toHaveAttribute('title', /isn't on a branch|has nothing committed|works on \S+ itself|no origin|isn't on a forge/)
})

test('what an agent shows opens beside the thread: an artifact in a sandbox, its versions, full view, comments to the agent, and images', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('draft').fill('Artifact please')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: Artifact please')).toBeVisible()
  // Made while the person watches, it opens by itself, in a frame that can reach nothing.
  const viewer = page.getByTestId('artifact-viewer')
  await expect(viewer).toBeVisible()
  await expect(viewer.getByTestId('artifact-frame')).toHaveAttribute('sandbox', 'allow-scripts')
  await expect(page.frameLocator('[data-testid="artifact-frame"]').getByRole('heading', { name: 'Rescan benchmark' })).toBeVisible()
  // The frame follows the viewer's theme (ADR 0071).
  await expect(page.frameLocator('[data-testid="artifact-frame"]').locator('html')).toHaveAttribute('data-theme', /^(light|dark)$/)
  await expect(page.getByTestId('artifact-card')).toBeVisible()
  // The version and the view are in the header, so nothing covers the page's top.
  const header = await viewer.getByRole('button', { name: 'Versions' }).locator('xpath=ancestor::div[contains(@class,"sticky")]').boundingBox()
  const frame = await viewer.getByTestId('artifact-frame').boundingBox()
  expect(frame!.y).toBeGreaterThanOrEqual(header!.y + header!.height)
  await viewer.getByRole('group', { name: 'View' }).getByRole('button', { name: 'Source' }).click()
  await expect(viewer.getByTestId('artifact-source')).toContainText('Rescan benchmark')
  await viewer.getByRole('group', { name: 'View' }).getByRole('button', { name: 'Preview' }).click()
  await expect(viewer.getByTestId('artifact-frame')).toBeVisible()

  await viewer.getByRole('button', { name: 'Full view (F)' }).click()
  await expect(page.getByLabel('draft')).toBeHidden()
  await viewer.getByRole('button', { name: 'Leave the full view (F)' }).click()
  await expect(page.getByLabel('draft')).toBeVisible()

  await page.getByLabel('draft').fill('Artifact again')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: Artifact again')).toBeVisible()
  await expect(viewer.getByRole('button', { name: 'Versions' })).toHaveText(/v2/)
  await viewer.getByRole('button', { name: 'Versions' }).click()
  await page.getByRole('menuitem', { name: /^v1/ }).click()
  await expect(viewer.getByRole('button', { name: 'Versions' })).toHaveText(/v1/)

  await viewer.getByRole('button', { name: 'Comments' }).click()
  await page.getByLabel('comment on the artifact').fill('Make the bars blue')
  await page.getByRole('button', { name: /^Send to the agent/ }).click()
  await expect(page.getByText(/Done: Comments on the artifact "Rescan benchmark"/)).toBeVisible()

  await page.getByLabel('draft').fill('Show me the pixel')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByTestId('shown-image').first()).toBeVisible()
  await viewer.getByRole('button', { name: 'Artifacts', exact: true }).click()
  const list = page.getByTestId('artifact-list')
  await expect(list).toContainText('2 versions')
  await expect(list.getByTestId('shown-image')).toBeVisible()
})

test("an artifact's page keeps what it stores for the viewer, and gets null for a capability it didn't declare", async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('draft').fill('Artifact counter')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: Artifact counter')).toBeVisible()
  const frame = page.frameLocator('[data-testid="artifact-frame"]')
  await expect(frame.getByText('Opened 1 times')).toBeVisible()
  await expect(frame.getByText('db: none')).toBeVisible()
  // The frame loads from its own address (ADR 0074), and what the page stored comes back on the next open (ADR 0072).
  await expect(page.getByTestId('artifact-frame')).toHaveAttribute('src', /^\/f\/[0-9a-f]{64}\/$/)
  const viewer = page.getByTestId('artifact-viewer')
  const reload = viewer.getByRole('button', { name: 'Reload' })
  if (await reload.isVisible()) await reload.click()
  else {
    await viewer.getByRole('button', { name: 'More' }).click()
    await page.getByRole('menuitem', { name: 'Reload' }).click()
  }
  await expect(frame.getByText('Opened 2 times')).toBeVisible()
})

test("an artifact's page keeps its tasks in the thread's page data, live and across reloads", async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('draft').fill('Artifact checklist')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: Artifact checklist')).toBeVisible()
  const frame = page.frameLocator('[data-testid="artifact-frame"]')
  await expect(frame.getByText('tasks: 0')).toBeVisible()
  await frame.getByRole('button', { name: 'Add a task' }).click()
  await expect(frame.getByText('tasks: 1')).toBeVisible()
  await frame.getByRole('button', { name: 'Add a task' }).click()
  await expect(frame.getByText('tasks: 2')).toBeVisible()
  await page.reload()
  // After a reload the panel either reopens the artifact or shows its card.
  const card = page.getByTestId('artifact-card').first()
  await expect(page.getByTestId('artifact-frame').or(card).first()).toBeVisible()
  if (await card.isVisible()) await card.click()
  await expect(page.frameLocator('[data-testid="artifact-frame"]').getByText('tasks: 2')).toBeVisible()
})

test('an agent that goes on by itself once its turn ended, as Claude does after a background task, opens a turn of its own', async ({ page }) => {
  await startInNewThread(page, 'Fake v2 agent')
  await page.getByLabel('draft').fill('add acp Later')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: add acp Later')).toBeVisible()
  await expect(page.getByText('Went on by itself.')).toBeVisible()
  await expect(page.getByText('without a prompt')).toHaveCount(0)
  await expect(page.getByTestId('agent-state')).toHaveText('Fake v2 agent is ready')
  // Its words are its own turn's reply, after the notice that opened it, not the turn before's.
  const turns = page.locator('[id^="turn-"]')
  await expect(turns).toHaveCount(2)
  await expect(turns.nth(1)).toContainText('Went on by itself.')
  await expect(turns.nth(0)).not.toContainText('Went on by itself.')
})

test("an agent's subagent and the command it left running show as cards, and the agent goes on once the command ends", async ({ page }) => {
  await startInNewThread(page, 'Fake v2 agent')
  await page.getByLabel('draft').fill('add acp Subagent Background')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: add acp Subagent Background')).toBeVisible()
  const cards = page.getByTestId('task')
  // The command runs on after the turn: its card stays out of the turn's fold, and it is listed above the composer until it ends.
  await expect(cards.filter({ hasText: 'sleep 1 && echo BG' })).toBeVisible()
  await expect(page.getByTestId('running-tasks')).toContainText('sleep 1 && echo BG')
  await expect(page.getByText('The background command printed BG.')).toBeVisible()
  await expect(page.getByTestId('running-tasks')).toBeHidden()
  // The subagent finished within the turn, so its card folded with the turn's work; its words are in it, not the thread's.
  await expect(page.getByText('Counted the lines.')).toBeHidden()
  await page.getByTestId('worked').first().locator(':scope > summary').click()
  const subagent = cards.filter({ hasText: 'Count lines' })
  await expect(subagent).toBeVisible()
  await subagent.getByRole('button', { expanded: false }).first().click()
  await expect(subagent).toContainText('Counted the lines.')
  await expect(subagent).toContainText('Reported back')
  await expect(subagent).toContainText('2 lines')
})

test('a command the agent left running in the background is stopped from above the composer', async ({ page }) => {
  await startInNewThread(page, 'Fake v2 agent')
  await page.getByLabel('draft').fill('add acp Keep')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: add acp Keep')).toBeVisible()
  const running = page.getByTestId('running-tasks')
  await expect(running).toContainText('keep-running')
  // Its card, out of the turn's fold while it runs, can stop it too.
  await expect(page.getByTestId('task').filter({ hasText: 'keep-running' }).getByRole('button', { name: 'Stop' })).toBeVisible()
  await running.getByRole('button', { name: /^Stop / }).click()
  await expect(running).toBeHidden()
  // Its end sets the agent going, in a turn of its own, which Silt stops before it says anything.
  await expect(page.locator('[id^="turn-"]')).toHaveCount(2)
  await expect(page.getByTestId('agent-state')).toHaveText('Fake v2 agent is ready')
  await expect(page.getByText('The kept command ended.')).toHaveCount(0)
})

test('prompts queue while the agent works, and a steer interrupts its turn', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('permissions').selectOption('ask_for_everything')
  const draft = page.getByLabel('draft')
  const permission = page.getByRole('group', { name: 'permission' })

  await draft.fill('first')
  await page.getByLabel('draft').press('Enter')
  await expect(permission).toBeVisible()
  await draft.fill('second')
  await page.getByLabel('draft').press('Enter')
  const queued = page.getByRole('region', { name: 'queued prompts' })
  await expect(queued).toContainText('second')
  await permission.getByRole('button', { name: 'Allow' }).click()
  await expect(page.getByText('Done: first')).toBeVisible()
  // The queue's next prompt went out when the turn ended.
  await expect(queued).toBeHidden()
  await expect(permission).toBeVisible()

  await draft.fill('change of plan')
  await page.getByRole('button', { name: 'Steer now' }).click()
  await expect(page.getByText('I was not allowed to edit.')).toBeVisible()
  // The cancelled turn's request can still show for a moment before the
  // steer's own replaces it; allowing the old one is refused, so try again.
  await expect(async () => {
    await page.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Allow' }).click({ timeout: 2000 })
    await expect(page.getByText('Done: I interrupted your last turn to say: change of plan')).toBeVisible({ timeout: 2000 })
  }).toPass({ timeout: 15_000 })
})

test('a thread started in the background from a branch plans its first prompt and waits in Needs you', async ({ page }) => {
  await page.goto(live.alice)
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'New thread', exact: true }).click()
  const form = page.getByRole('form', { name: 'new thread' })
  await form.getByLabel('first prompt').fill('write the docs')
  await form.getByRole('radio', { name: 'Worktree' }).click()
  await form.getByLabel('base branch').selectOption('origin/main')
  await form.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await form.getByLabel('permissions').selectOption('plan')
  await startInTheBackground(page)

  await expect(page.getByText('Fake agent started in a new thread')).toBeVisible()
  await expect(form.getByLabel('first prompt')).toHaveValue('')
  const waiting = page.getByRole('region', { name: 'needs you' })
  await waiting.getByRole('group').first().getByRole('link').first().click()
  await expect(page.getByText('Plan: write the docs')).toBeVisible()
  await expect(page.getByTestId('agent-state')).toHaveText('Fake agent wrote a plan for you to approve')
  await expect(page.getByLabel('permissions', { exact: true })).toHaveValue('plan')
})

test("the agent's modes and model are offered, and the Plan preset plans in its plan mode", async ({ page }) => {
  // Wide enough for the agent's chips to stand in the row beside the right panel, rather than fold into one.
  await page.setViewportSize({ width: 1600, height: 900 })
  await startInNewThread(page, 'Fake agent')
  const mode = page.getByLabel('mode', { exact: true })
  await expect(mode).toHaveValue('code')
  await page.getByLabel('model').selectOption({ label: 'Smart' })
  await expect(page.getByLabel('model')).toHaveValue('smart')

  await page.getByLabel('draft').fill('add a parser')
  await page.getByLabel('permissions').selectOption('plan')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Plan: add a parser')).toBeVisible()
  await expect(page.getByTestId('agent-state')).toHaveText('Fake agent wrote a plan for you to approve')
  await expect(mode).toHaveValue('plan')
  await expect(page.getByRole('region', { name: 'needs you' })).toBeVisible()

  await page.getByRole('group', { name: 'plan' }).getByRole('button', { name: 'Approve plan' }).click()
  await expect(page.getByText('Done: The plan is approved. Go ahead.')).toBeVisible()
  await expect(mode).toHaveValue('code')
  await expect(page.getByTestId('agent-state')).toHaveText('Fake agent is ready')
})

test('the Plan preset asks an agent without a plan mode for a plan by the prompt', async ({ page }) => {
  await startInNewThread(page, 'Plain agent')
  await expect(page.getByLabel('mode', { exact: true })).toHaveCount(0)
  await page.getByLabel('draft').fill('add a lexer')
  await page.getByLabel('permissions').selectOption('plan')
  await page.getByLabel('draft').press('Enter')
  // This agent edits anyway; under Plan an edit asks first.
  const allow = () => page.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Allow' }).click()
  await allow()
  await expect(page.getByText(/^Done: Before you change anything, write a plan[\s\S]*add a lexer$/)).toBeVisible()
  await expect(page.getByTestId('agent-state')).toHaveText('Plain agent wrote a plan for you to approve')

  await page.getByLabel('draft').fill('use a table instead')
  await page.getByLabel('draft').press('Enter')
  await allow()
  await expect(page.getByText('Done: use a table instead')).toBeVisible()
  await expect(page.getByTestId('agent-state')).toHaveText('Plain agent is ready')
})

const SHOT = { name: 'shot.png', mimeType: 'image/png', buffer: Buffer.from([0x89, 0x50, 0x4e, 0x47]) }
const NOTES = { name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('hello') }

test('files attached to a prompt reach the agent and are named in the conversation', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('attach files').setInputFiles([SHOT, NOTES])
  await expect(page.getByRole('list', { name: 'attached files' })).toContainText('shot.png')
  await page.getByLabel('draft').fill('look at these')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: look at these (with an image (image/png), attachment:notes.txt saying "hello")')).toBeVisible()
  await expect(page.getByText(/\(attached: prompt\/\w+\/shot\.png, prompt\/\w+\/notes\.txt\)/)).toBeVisible()
  await expect(page.getByRole('list', { name: 'attached files' })).toBeHidden()
})

test("an agent that takes no images is told a text file's content in the prompt, and an image is refused", async ({ page }) => {
  await startInNewThread(page, 'Plain agent')
  await page.getByLabel('attach files').setInputFiles([SHOT])
  await expect(page.getByText("Plain agent doesn't take files like shot.png")).toBeVisible()
  await expect(page.getByRole('list', { name: 'attached files' })).toBeHidden()

  await page.getByLabel('draft').fill('see this')
  await page.getByLabel('attach files').setInputFiles([NOTES])
  await page.getByLabel('draft').press('Enter')
  // The message is Markdown: the file's content shows as a code block.
  await expect(page.getByText(/^Done: see this\s*notes\.txt:\s*hello$/)).toBeVisible()
})

// A screenshot on the clipboard is named `image.png`, whatever it shows.
const SCREENSHOT = { ...SHOT, name: 'image.png' }

type Sent = { name: string; mimeType: string; buffer: Buffer }

/** Sends `event` with `files` to the composer labelled `label`, as a paste or a drop does. */
async function sendFiles(page: Page, label: string, event: 'paste' | 'drop', files: Sent[]) {
  const sent = files.map((file) => ({ name: file.name, type: file.mimeType, bytes: [...file.buffer] }))
  await page.getByLabel(label, { exact: true }).evaluate(
    (box, { event, sent }) => {
      const data = new DataTransfer()
      for (const file of sent) data.items.add(new File([new Uint8Array(file.bytes)], file.name, { type: file.type }))
      const init = { bubbles: true, cancelable: true }
      box.dispatchEvent(event === 'paste' ? new ClipboardEvent('paste', { ...init, clipboardData: data }) : new DragEvent('drop', { ...init, dataTransfer: data }))
    },
    { event, sent },
  )
}

test('an image pasted on New thread, and a file attached there, go with the first prompt', async ({ page }) => {
  await page.goto(live.alice)
  await page.getByRole('radio', { name: 'Worktree' }).click()
  await page.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await page.getByLabel('first prompt').fill('look at this')
  await sendFiles(page, 'first prompt', 'paste', [SCREENSHOT])
  await page.getByLabel('attach files').setInputFiles([NOTES])
  const attached = page.getByRole('list', { name: 'attached files' })
  await expect(attached).toContainText('pasted-1.png')
  await expect(attached).toContainText('notes.txt')
  await page.getByLabel('first prompt').press('Enter')
  await expect(page.getByText('Done: look at this (with an image (image/png), attachment:notes.txt saying "hello")')).toBeVisible()
  await expect(page.getByText(/\(attached: prompt\/\w+\/pasted-1\.png, prompt\/\w+\/notes\.txt\)/)).toBeVisible()
})

test("files pasted and dropped into a running thread's composer reach the agent", async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await sendFiles(page, 'draft', 'paste', [SCREENSHOT])
  await sendFiles(page, 'draft', 'drop', [NOTES])
  const attached = page.getByRole('list', { name: 'attached files' })
  await expect(attached).toContainText('pasted-1.png')
  await expect(attached).toContainText('notes.txt')
  await page.getByLabel('draft').fill('and these')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: and these (with an image (image/png), attachment:notes.txt saying "hello")')).toBeVisible()
  await expect(attached).toBeHidden()
})

test('an image pasted for an agent known to take none is refused at once', async ({ page }) => {
  await page.goto(live.alice)
  await page.getByLabel('agent to start').selectOption({ label: 'Plain agent' })
  // Known once the daemon has asked the agent what it offers.
  await page.getByRole('button', { name: 'add to the prompt' }).click()
  await expect(page.getByRole('menuitem', { name: 'Attach text files (Plain agent takes no images)' })).toBeVisible({ timeout: 30_000 })
  await page.keyboard.press('Escape')
  await sendFiles(page, 'first prompt', 'paste', [SCREENSHOT])
  await expect(page.getByText("Plain agent doesn't take files like pasted-1.png")).toBeVisible()
  await expect(page.getByRole('list', { name: 'attached files' })).toBeHidden()
})

test("a review of the agent's changes goes back to it as one prompt", async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('draft').fill('add a function')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: add a function')).toBeVisible()
  await page.getByLabel('draft').fill('Rewrite main')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: Rewrite main')).toBeVisible()

  const changes = page.getByRole('region', { name: 'changes' })
  await page.getByLabel('compare').selectOption({ label: 'Last turn' })
  await expect(changes.getByTestId('diff')).toContainText('+ fn acp() {}')
  // Comments on a deleted line and on an added one wait in the review.
  await changes.getByRole('button', { name: /^comment on src\/main\.rs deleted line \d+$/ }).first().click()
  await page.getByLabel('code comment').fill('keep this one')
  await page.getByRole('button', { name: 'Start a review' }).click()
  await changes.getByRole('button', { name: /^comment on src\/main\.rs line \d+$/ }).last().click()
  await page.getByLabel('code comment').fill('give it a better name')
  await page.getByRole('button', { name: 'Add review comment' }).click()
  await expect(changes.getByRole('group', { name: 'pending comment' })).toHaveCount(2)

  const review = changes.getByRole('group', { name: 'review' })
  await expect(review).toContainText('2 pending')
  await review.getByRole('button', { name: 'Finish review' }).click()
  const finish = page.getByRole('dialog', { name: 'finish review' })
  await finish.getByLabel('review note').fill('Nearly there.')
  await expect(finish.getByRole('radio', { name: 'Request changes' })).toBeChecked()
  await finish.getByRole('button', { name: 'Request changes', exact: true }).click()
  await expect(review).toBeHidden()
  await expect(
    page.getByTestId('entry').filter({
      hasText: /Done: Nearly there\.\s*Please address these review comments:\s*src\/main\.rs:1, lines the change deleted: keep this one/,
    }),
  ).toBeVisible()

  // Both are comments now, sent; the deleted line's shows beside it against the thread's start.
  await page.getByLabel('compare').selectOption({ label: 'This thread' })
  await expect(changes.getByText('alice: keep this one')).toBeVisible()
  await expect(changes.getByText('alice: give it a better name')).toBeVisible()
  await expect(changes.getByText('Sent', { exact: true })).toHaveCount(2)

  await page.getByLabel('compare').selectOption({ label: 'Against main' })
  await expect(changes.getByTestId('diff')).toContainText('+ fn acp() {}')
})

test('the review expands across the page, split, and its keys move through the diff', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('draft').fill('add a function')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: add a function')).toBeVisible()
  const thread = new URL(page.url()).pathname.split('/')[2]
  // Keys without a modifier are the composer's while it has the focus.
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())

  await page.keyboard.press('Shift+D')
  await expect(page).toHaveURL(new RegExp(`/t/${thread}/review$`))
  const changes = page.getByRole('region', { name: 'changes' })
  await expect(changes.getByTestId('diff')).toContainText('fn acp() {}')
  await expect(page.getByRole('tablist', { name: 'layout' }).getByRole('tab', { name: 'Split' })).toHaveAttribute('aria-selected', 'true')
  // The conversation's composer isn't part of it.
  await expect(page.getByLabel('draft')).toHaveCount(0)
  await expect(page.getByRole('navigation', { name: 'files' })).toContainText('src/main.rs')

  // n puts the cursor on the first hunk, and c opens a comment on what it changed; Esc there only closes the box.
  await page.keyboard.press('n')
  await expect(changes.locator('[data-hunk="src/main.rs#0"]')).toHaveClass(/ring-accent/)
  await page.keyboard.press('c')
  await expect(page.getByLabel('code comment')).toBeFocused()
  await page.getByLabel('code comment').fill('a better name')
  await page.getByRole('button', { name: 'Start a review' }).click()
  await expect(page.getByRole('group', { name: 'review' })).toContainText('1 pending')

  // s switches the layout; Esc goes back to the conversation, the pending comment kept.
  await page.keyboard.press('s')
  await expect(page.getByRole('tablist', { name: 'layout' }).getByRole('tab', { name: 'Unified' })).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('Escape')
  await expect(page).toHaveURL(new RegExp(`/t/${thread}$`))
  await expect(page.getByLabel('draft')).toBeVisible()
  await expect(page.getByRole('region', { name: 'changes' }).getByRole('group', { name: 'pending comment' })).toHaveCount(1)
  await page.getByRole('region', { name: 'changes' }).getByRole('group', { name: 'pending comment' }).getByRole('button', { name: 'delete' }).click()
  // Away from the review, n is New thread again.
  await page.keyboard.press('n')
  await expect(page).not.toHaveURL(/\/t\//)
})

test("the review names the turn that wrote each hunk, and shows one turn's changes alone", async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('draft').fill('add a function')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: add a function')).toBeVisible()
  await page.getByLabel('draft').fill('Rewrite main')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: Rewrite main')).toBeVisible()
  const thread = new URL(page.url()).pathname.split('/')[2]
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  await page.keyboard.press('Shift+D')

  const changes = page.getByRole('region', { name: 'changes' })
  await expect(changes.locator('[data-hunk]').first()).toContainText(/turns? \d/)
  const turns = page.getByRole('group', { name: 'turns' })
  await turns.getByRole('button', { name: '1', exact: true }).click()
  await expect(turns.getByRole('button', { name: '1', exact: true })).toHaveAttribute('aria-pressed', 'true')
  await expect(changes.locator('[data-hunk]').first()).toContainText('turn 1 · add a function')
  await expect(changes.getByText(/^turn 2/)).toHaveCount(0)
  await page.getByRole('button', { name: 'All changes' }).click()
  await expect(page.getByLabel('compare')).toBeVisible()

  // t opens the hunk's turn in the conversation.
  await page.keyboard.press('n')
  await page.keyboard.press('t')
  await expect(page).toHaveURL(new RegExp(`/t/${thread}#turn-\\d+$`))
})

test('a file marked seen comes back when the agent changes it, and the next review starts after the turn reviewed', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('draft').fill('add a function')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: add a function')).toBeVisible()
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  await page.keyboard.press('Shift+D')

  const files = page.getByRole('navigation', { name: 'files' })
  const main = files.getByRole('checkbox', { name: 'seen src/main.rs' })
  await main.check()
  await expect(page.getByTestId('seen-count')).toHaveText('1/1 seen')
  // Nothing to change: the review is marked reviewed through the turn.
  await page.getByRole('button', { name: 'Finish review' }).click()
  await page.getByRole('dialog', { name: 'finish review' }).getByRole('button', { name: 'Mark reviewed' }).click()
  await expect(page.getByText('Reviewed through turn 1', { exact: true })).toBeVisible()

  await page.getByRole('button', { name: /^Conversation/ }).click()
  await page.getByLabel('draft').fill('Rewrite main')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: Rewrite main')).toBeVisible()
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  await page.keyboard.press('Shift+D')

  // The review starts after the turn reviewed, and the file seen before comes back half-ticked.
  await expect(page.getByRole('group', { name: 'turns' }).getByRole('button', { name: '2', exact: true })).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByRole('group', { name: 'turns' }).getByRole('button', { name: '1', exact: true })).toHaveAttribute('aria-pressed', 'false')
  await expect(files).toContainText(/new since you saw it|changed since you saw it/)
  await expect(main).not.toBeChecked()
  await expect(page.getByTestId('seen-count')).toHaveText('0/1 seen')
  // v ticks it off again.
  await page.keyboard.press('j')
  await page.keyboard.press('v')
  await expect(page.getByTestId('seen-count')).toHaveText('1/1 seen')
})

/** Starts the fake agent in a new thread of the project whose name matches, asking before everything. */
async function startInBackground(page: Page, project: RegExp, prompt: string) {
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'New thread', exact: true }).click()
  const form = page.getByRole('form', { name: 'new thread' })
  await pickProject(page, project)
  await form.getByLabel('first prompt').fill(prompt)
  await form.getByRole('radio', { name: 'Worktree' }).click()
  await form.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await form.getByLabel('permissions').selectOption('ask_for_everything')
  await startInTheBackground(page)
  await expect(form.getByLabel('first prompt')).toHaveValue('')
}

test('two agents in two projects queue prompts, are answered from the Needs you queue, and a fork is offered back', async ({ page }) => {
  await page.goto(live.alice)
  await startInBackground(page, /origin/, 'work in origin')
  await startInBackground(page, /tools/, 'work in tools')

  const sidebar = page.getByRole('navigation', { name: 'projects' })
  const origin = sidebar.getByRole('group', { name: 'work in origin needs you' })
  const tools = sidebar.getByRole('group', { name: 'work in tools needs you' })
  await expect(origin).toContainText('asks to run')
  await expect(tools).toContainText('asks to run')

  await origin.getByRole('link', { name: /^work in origin/ }).click()
  await page.getByLabel('draft').fill('then document it')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByRole('region', { name: 'queued prompts' })).toContainText('then document it')

  // Each is answered from the queue, which lists both and shows the one chosen.
  await sidebar.getByRole('link', { name: /Open the queue/ }).click()
  const queue = page.getByRole('region', { name: 'waiting on you' })
  await queue.getByRole('button', { name: /^work in tools/ }).click()
  await queue.getByRole('group', { name: 'work in tools needs you' }).getByRole('button', { name: 'Allow' }).click()
  await expect(tools).toBeHidden()
  // What was answered leaves the queue, and the one left is shown in its place.
  const there = queue.getByRole('group', { name: 'work in origin needs you' })
  await expect(there).toContainText('Edit src/main.rs')
  await there.getByRole('button', { name: 'Allow' }).click()
  await sidebar.getByRole('link', { name: /^work in origin/ }).first().click()
  await expect(page.getByText('Done: work in origin')).toBeVisible()
  // The queued prompt went next, and asks in turn.
  await page.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Allow' }).click()
  await expect(page.getByText('Done: then document it')).toBeVisible()
  await expect(origin).toBeHidden()

  await page.getByRole('button', { name: 'Fork', exact: true }).click()
  await expect(page.getByRole('main').getByText(/^Fork of/)).toBeVisible()
  await page.getByRole('button', { name: 'Offer as merge request' }).first().click()
  await page.getByLabel('merge request title').fill('Origin work')
  await page.getByRole('button', { name: 'Offer', exact: true }).click()
  await expect(page).toHaveURL(/\/mr\//)
  await expect(page.getByTestId('mr-status')).toHaveText('open')

  // A change guide: by hand, then by the fork's agent once one runs there.
  const guide = page.getByRole('region', { name: 'change guide' })
  await guide.getByRole('button', { name: "Ask the fork's agent" }).click()
  await expect(page.getByRole('main')).toContainText('start an agent in the fork')
  await guide.getByRole('button', { name: 'Write it' }).click()
  await guide.getByLabel('guide').fill('Start with src/main.rs.')
  await guide.getByRole('button', { name: 'Keep guide' }).click()
  await expect(guide.getByTestId('guide')).toHaveText('Start with src/main.rs.')
  await expect(guide).toContainText('by alice')

  const request = page.url()
  await page.getByRole('main').getByRole('link', { name: 'Open the fork' }).click()
  await page.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByTestId('agent-state')).toHaveText('Fake agent is ready')
  await page.goto(request)
  await guide.getByRole('button', { name: "Ask the fork's agent" }).click()
  // The fake agent answers with its whole prompt, primed with the conversation.
  await expect(guide.getByTestId('guide')).toContainText('Write a change guide for the reviewers', { timeout: 15_000 })
  await expect(guide).toContainText('by Fake agent')

  await page.getByRole('link', { name: 'Merge requests' }).click()
  await expect(page.getByRole('main')).toContainText('Origin work')
})

test('projects show their clone here, and one is added by cloning its URL', async ({ page }) => {
  await page.goto(live.alice)
  await settings(page, 'Projects')
  const list = page.getByTestId('projects')
  // A clone's path is cut to its last folders, whole in its title.
  await expect(list.getByRole('listitem', { name: 'tools' })).toContainText('Cloned here at …/alice/tools')
  await expect(list.getByRole('listitem', { name: 'tools' }).getByTitle(`${live.work}/alice/tools`, { exact: true })).toBeVisible()
  const origin = list.getByRole('listitem', { name: 'origin' })
  await expect(origin).toContainText(/\d+ threads?/)
  await expect(origin.getByTitle(`${live.work}/alice/repo`, { exact: true })).toBeVisible()
  // The first project opens to its defaults.
  await expect(origin.getByRole('group', { name: 'defaults for origin' }).getByLabel('Default agent')).toHaveValue('plain')

  await page.getByRole('main').getByRole('link', { name: 'Add a project' }).click()
  const add = page.getByRole('region', { name: 'add a project' })
  await add.getByLabel('clone path').fill(live.work)
  await add.getByRole('button', { name: 'Add', exact: true }).click()
  await expect(add).toContainText("isn't in a git repository; create it as a new project instead")

  await add.getByLabel('remote URL').fill(`${live.work}/docs.git`)
  await add.getByLabel('where to clone it').fill(`${live.work}/alice/docs`)
  await add.getByRole('button', { name: 'Clone', exact: true }).click()
  await expect(add).toContainText(`${live.work}/alice/docs`)
  await page.getByRole('link', { name: 'Done' }).click()
  await expect(list.getByRole('listitem', { name: 'docs' }).getByTitle(`${live.work}/alice/docs`, { exact: true })).toBeVisible()
  expect(readFileSync(join(live.work, 'alice', 'docs', 'src', 'main.rs'), 'utf8')).toBe('fn tool() {}\n')
})

test("a new thread starts with what its project says, until the person picks", async ({ page }) => {
  await page.goto(live.alice)
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'New thread', exact: true }).click()
  const form = page.getByRole('form', { name: 'new thread' })
  const pick = (project: RegExp) => pickProject(page, project)
  await pick(/origin/)
  await expect(form.getByLabel('agent to start')).toHaveValue('plain')
  // Without a project default, the first agent is picked.
  await pick(/tools/)
  await expect(form.getByLabel('agent to start')).not.toHaveValue('plain')
  await form.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await pick(/origin/)
  await expect(form.getByLabel('agent to start')).toHaveValue('fake')
})

test('each person pins, snoozes and settles threads for themselves, and sees what changed since they looked', async ({ browser }) => {
  const alice = await (await browser.newContext()).newPage()
  const bob = await (await browser.newContext()).newPage()
  await alice.goto(live.alice)
  await bob.goto(live.bob)
  const sidebar = (page: Page) => page.getByRole('navigation', { name: 'projects' })
  const shared = (page: Page) => sidebar(page).getByRole('link', { name: /^Add a farewell/ }).first()
  const act = async (page: Page, action: string) => {
    await shared(page).hover()
    await sidebar(page).getByRole('button', { name: 'actions for Add a farewell' }).first().click()
    await page.getByRole('menuitem', { name: action }).click()
  }

  await act(alice, 'Pin')
  await expect(sidebar(alice).getByRole('region', { name: 'pinned' })).toContainText('Add a farewell')
  await expect(sidebar(bob).getByRole('region', { name: 'pinned' })).toHaveCount(0)
  await act(alice, 'Unpin')
  await act(alice, 'Snooze until something happens')
  // Snoozed threads fold behind a count under their project.
  const more = sidebar(alice).getByRole('button', { name: 'Show 1 snoozed' })
  await more.click()
  await act(alice, 'Wake up')
  await expect(more).toHaveCount(0)

  // Bob looks, then Alice edits: Bob sees there's something new.
  await shared(bob).click()
  await expect(bob.getByText('Please add a farewell line to main').first()).toBeVisible()
  await sidebar(bob).getByRole('link', { name: 'All threads' }).click()
  await expect(shared(bob).getByRole('img', { name: 'unread' })).toHaveCount(0)
  await openFile(alice, live.alice)
  await alice.getByTestId('editor').locator('.cm-content').click()
  await alice.keyboard.press('ControlOrMeta+End')
  await alice.keyboard.type('fn later() {}\n')
  await expect(shared(bob).getByRole('img', { name: 'unread' })).toBeVisible({ timeout: 20_000 })
  await shared(bob).click()
  await expect(shared(bob).getByRole('img', { name: 'unread' })).toHaveCount(0)

  await act(alice, 'Settle')
  // Settled, it folds with the rest; the fold is still open from waking it above.
  await expect(sidebar(alice).getByRole('button', { name: /^Show (fewer|\d+ more)$/ }).first()).toBeVisible()
  await alice.context().close()
  await bob.context().close()
})

test("an org's owners set it up in the web UI, and its members only see it", async ({ browser }) => {
  const data = join(live.work, 'server')
  execFileSync(SILT, ['server', 'user', 'admin', 'alice', '--data', data])
  const alice = await (await browser.newContext()).newPage()
  await alice.goto(live.alice)
  await settings(alice, 'Orgs')
  await alice.getByLabel('org name').fill('acme')
  await alice.getByRole('button', { name: 'Create org' }).click()
  const acme = alice.getByRole('region', { name: 'acme', exact: true })
  await expect(acme).toContainText('owner')

  await acme.getByLabel('user to add').fill('bob')
  await acme.getByRole('button', { name: 'Add member' }).click()
  await expect(acme.getByRole('list', { name: 'members of acme' })).toContainText('bob')
  await acme.getByLabel('team name').fill('core')
  await acme.getByRole('button', { name: 'Create team' }).click()
  const core = acme.getByRole('listitem', { name: 'team core' })
  await core.getByRole('button', { name: '+ bob' }).click()
  await expect(core).toContainText('bob')
  await acme.getByLabel('default visibility of acme').selectOption('team:core')
  await expect(acme.getByLabel('default visibility of acme')).toHaveValue('team:core')
  execFileSync(SILT, ['server', 'user', 'admin', 'alice', '--revoke', '--data', data])

  const bob = await (await browser.newContext()).newPage()
  await bob.goto(live.bob)
  await settings(bob, 'Orgs')
  const seen = bob.getByRole('region', { name: 'acme', exact: true })
  await expect(seen).toContainText('member')
  await expect(seen.getByLabel('default visibility of acme')).toBeDisabled()
  await expect(seen.getByRole('button', { name: 'Add member' })).toHaveCount(0)
  await alice.context().close()
  await bob.context().close()
})

test('the owner shares a thread from its page: who sees it, and who may write into it', async ({ page }) => {
  await page.goto(live.alice)
  await allThreads(page)
  await page.getByRole('link', { name: live.thread }).click()
  await page.getByRole('button', { name: 'Share' }).click()
  const dialog = page.getByRole('dialog', { name: /^Share / })
  const collaborators = dialog.getByRole('list', { name: 'collaborators' })
  await expect(collaborators).toContainText('bob')

  await collaborators.getByRole('button', { name: 'Remove' }).click()
  await expect(collaborators).toContainText('Nobody yet.')
  await dialog.getByLabel('person to add').fill('bob')
  await dialog.getByRole('button', { name: 'Add', exact: true }).click()
  await expect(collaborators).toContainText('bob')

  const who = dialog.getByRole('group', { name: 'who sees it' })
  await who.getByRole('radio', { name: /^Only alice/ }).check()
  await expect(who.getByRole('radio', { name: /^Only alice/ })).toBeChecked()
  await who.getByRole('radio', { name: /^Everyone in the org/ }).check()
  await expect(who.getByRole('radio', { name: /^Everyone in the org/ })).toBeChecked()

  await dialog.getByLabel('person to add').fill('nobody-by-that-name')
  await dialog.getByRole('button', { name: 'Add', exact: true }).click()
  await expect(dialog).toContainText('could not share the thread')
})

// Last: it commits in Alice's clone, which the threads above work in.
test("a thread's page lists its commits with the turns they came in", async ({ page }) => {
  const repo = join(live.work, 'alice', 'repo')
  execFileSync('git', ['-C', repo, '-c', 'user.name=Alice', '-c', 'user.email=alice@example.com', 'commit', '-qam', 'Add the farewell'])
  await page.goto(live.alice)
  const commits = page.getByRole('region', { name: 'commits' })
  await expect(async () => {
    await page.goto(new URL(`/t/${live.thread}`, live.alice).toString())
    // Commits are under Details; the panel opens on Changes.
    await page.getByRole('tab', { name: 'Details' }).click()
    await expect(commits).toContainText('Add the farewell', { timeout: 2_000 })
  }).toPass({ timeout: 20_000 })
  // Tests above prompted in this thread too, so which turn depends on them.
  await expect(commits.getByRole('link', { name: /^made in turn \d+$/ }).last()).toBeVisible()
})

test('the graph shows turns, commits, forks, requests and merges, and opens a turn from its details', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await stopAgent(page)
  await page.getByRole('button', { name: 'Fork', exact: true }).click()
  await expect(page.getByRole('main').getByText(/^Fork of/)).toBeVisible()
  await page.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await page.getByLabel('draft').press('Enter')
  await page.getByLabel('draft').fill('work to merge')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText(/^Done: [\s\S]*work to merge$/)).toBeVisible()
  await stopAgent(page)
  await page.getByRole('button', { name: 'Offer as merge request' }).first().click()
  await page.getByLabel('merge request title').fill('Work to merge')
  await page.getByRole('button', { name: 'Offer', exact: true }).click()
  await page.getByRole('button', { name: 'Approve and merge' }).click()
  await expect(page.getByTestId('mr-status')).toHaveText('approved')

  await allThreads(page)
  await page.getByRole('main').getByRole('link', { name: 'Graph' }).click()
  // A test above settles threads, which the graph leaves out unless asked.
  await page.getByLabel('Settled threads').check()
  const graph = page.getByRole('list', { name: 'graph' })
  await expect(graph.getByRole('button', { name: /^fork forked from/ }).first()).toBeVisible()
  // This test's own request: the earlier tests' turns can push theirs past the first page.
  await expect(graph.getByRole('button', { name: /^request Work to merge/ })).toBeVisible()
  await expect(graph.getByRole('button', { name: /^merge / }).first()).toBeVisible()

  // The fixture's commit is among the oldest rows, which the turns of the
  // tests above can push past the first page until only commits are shown.
  await page.getByLabel('Turns', { exact: true }).uncheck()
  await expect(graph.getByRole('button', { name: /^turn/ })).toHaveCount(0)
  await graph.getByRole('button', { name: /^commit Add the farewell/ }).click()
  const details = page.getByRole('complementary', { name: 'details' })
  await details.getByRole('link', { name: /^Turn \d+ of thread/ }).click()
  await expect(page).toHaveURL(new RegExp(`/t/[0-9a-f]*${live.thread}#turn-\\d+$`))
  await expect(page.locator('[data-testid="entry"][data-highlighted]')).toBeInViewport()
})

test('a project kept on this machine starts threads that stay here, until it moves to the server', async ({ page }) => {
  await page.goto(live.alice)
  const toProjects = () => settings(page, 'Projects')
  await toProjects()
  const home = page.getByTestId('projects').getByRole('listitem', { name: 'tools' }).getByLabel('where tools syncs')
  await expect(home).toHaveValue(live.server)
  await home.selectOption({ label: 'This machine only' })
  await expect(page.getByText(/stay on this machine only/).first()).toBeVisible()

  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'New thread', exact: true }).click()
  const form = page.getByRole('form', { name: 'new thread' })
  await pickProject(page, /tools/)
  await form.getByRole('radio', { name: 'Worktree' }).click()
  await form.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await form.getByLabel('first prompt').press('Enter')
  await expect(page.getByTestId('agent-state')).toHaveText('Fake agent is ready')
  await expect(page.getByText('This machine only', { exact: true })).toBeVisible()
  await stopAgent(page)
  const kept = new URL(page.url()).pathname

  await toProjects()
  await home.selectOption(live.server)
  await expect(page.getByText(/1 threads kept on this machine now sync there/).first()).toBeVisible()
  await page.goto(new URL(kept, live.alice).toString())
  await expect(page.getByPlaceholder('Ask for the next step; sending starts the agent')).toBeVisible()
  await expect(page.getByText('This machine only', { exact: true })).toHaveCount(0)
})

test('a new project made from New thread starts its first thread in a folder made for it, kept on this machine', async ({ page }) => {
  await page.goto(live.alice)
  await page.getByRole('button', { name: /^project:/ }).click()
  const picker = page.getByRole('dialog', { name: 'Pick a project' })
  await picker.getByRole('tab', { name: 'New' }).click()
  await picker.getByLabel('project name').fill('scratch-app')
  await picker.getByLabel('in the folder').fill(live.work)
  await expect(picker).toContainText(`Makes ${live.work}/scratch-app with an empty git repository`)
  await picker.getByRole('button', { name: 'Create' }).click()
  await expect(page.getByRole('button', { name: 'project: scratch-app' })).toBeVisible()

  const form = page.getByRole('form', { name: 'new thread' })
  await expect(form.getByRole('radio', { name: 'Local' })).toHaveAttribute('aria-checked', 'true')
  await form.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await form.getByLabel('first prompt').press('Enter')
  await expect(page.getByTestId('agent-state')).toHaveText('Fake agent is ready')
  await expect(page.getByText('This machine only', { exact: true })).toBeVisible()
  await page.getByLabel('draft').fill('hello')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: hello')).toBeVisible()
  await stopAgent(page)
})

test("the agent's commands are offered as / starts a prompt, in its thread and in a new one", async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  const draft = page.getByLabel('draft')
  const commands = page.getByRole('listbox', { name: 'commands' })
  await draft.fill('/co')
  await expect(commands.getByRole('option')).toHaveText(['/count <what to count>Count what you name', '/compactSummarize the conversation so far'])
  await draft.press('ArrowDown')
  await expect(commands.getByRole('option').filter({ hasText: '/compact' })).toHaveAttribute('aria-selected', 'true')
  await draft.press('ArrowUp')
  await draft.press('Enter')
  await expect(draft).toHaveValue('/count ')
  await expect(commands).toBeHidden()
  await draft.pressSequentially('lines')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: /count lines')).toBeVisible()

  await draft.fill('/')
  await expect(commands.getByRole('option')).toHaveCount(2)
  await draft.press('Escape')
  await expect(commands).toBeHidden()
  await stopAgent(page)

  // A new thread offers them before the agent runs, as it last offered them here.
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'New thread', exact: true }).click()
  const form = page.getByRole('form', { name: 'new thread' })
  await form.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await form.getByLabel('first prompt').fill('/comp')
  await commands.getByRole('option').filter({ hasText: '/compact' }).click()
  await expect(form.getByLabel('first prompt')).toHaveValue('/compact ')
})

test('a deck pages by keys, links to a slide and shows the presenter its notes (ADR 0076)', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('draft').fill('Artifact deck')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: Artifact deck')).toBeVisible()
  const card = page.getByTestId('artifact-card').first()
  if (!(await page.getByTestId('slides').isVisible())) await card.click()
  const slides = page.getByTestId('slides')
  await expect(slides.getByLabel('slide 1 of 2')).toBeVisible()
  await expect(page.frameLocator('[data-testid="slide-frame"]').getByText('Rescans got 3x faster')).toBeVisible()
  await slides.click({ position: { x: 5, y: 5 } })
  await page.keyboard.press('ArrowRight')
  await expect(slides.getByLabel('slide 2 of 2')).toBeVisible()
  await slides.getByRole('button', { name: '1. Rescans' }).click()
  await slides.getByRole('button', { name: 'Present' }).click()
  await expect(page.getByTestId('presenter')).toContainText('Open with the headline.')
  await expect(page.getByTestId('presenter')).toContainText('Why')
})

test('a data page links each number to its source, and a canvas shows each board at its size (ADR 0076)', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('draft').fill('Artifact numbers')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: Artifact numbers')).toBeVisible()
  if (!(await page.getByTestId('data-page').isVisible())) await page.getByTestId('artifact-card').first().click()
  const data = page.getByTestId('data-page')
  await expect(data.getByTestId('stat')).toContainText('412')
  await expect(data.getByRole('link', { name: 'source: silt-bench rescan' }).first()).toHaveAttribute('href', '#source-bench')
  await data.getByRole('button', { name: 'Table' }).click()
  await expect(data.getByRole('cell', { name: '1,080' })).toBeVisible()

  await page.getByLabel('draft').fill('Artifact boards')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: Artifact boards')).toBeVisible()
  const cards = page.getByTestId('artifact-card')
  await cards.last().click()
  const boards = page.getByTestId('board')
  await expect(boards).toHaveCount(3)
  await expect(boards.nth(2)).toContainText('390×844')
})

test('a doc and a sheet are edited in place, and what was typed is kept (ADR 0077)', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('draft').fill('Artifact doc')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: Artifact doc')).toBeVisible()
  if (!(await page.getByTestId('doc-editor').isVisible())) await page.getByTestId('artifact-card').first().click()
  const doc = page.getByTestId('doc-editor')
  await expect(doc.getByRole('heading', { name: 'Plan' })).toBeVisible()
  await doc.getByRole('button', { name: 'Edit' }).click()
  const text = doc.getByLabel("the doc's text")
  await text.press('ControlOrMeta+End')
  await text.pressSequentially('- review\n')
  await expect(doc.getByText('Saved')).toBeVisible()
  await doc.getByRole('button', { name: 'Read' }).click()
  await expect(doc.getByText('review')).toBeVisible()

  await page.getByLabel('draft').fill('Artifact sheet')
  await page.getByLabel('draft').press('Enter')
  await expect(page.getByText('Done: Artifact sheet')).toBeVisible()
  await page.getByRole('button', { name: 'Open the artifact Tracker, version 1' }).click()
  const sheet = page.getByTestId('sheet')
  await expect(sheet.getByRole('grid')).toContainText('apples')
  await sheet.getByRole('row').nth(3).getByRole('gridcell').nth(1).dblclick()
  await sheet.getByLabel('cell B3').fill('=SUM(B1:B2)')
  await sheet.getByLabel('cell B3').press('Enter')
  await expect(sheet.getByRole('row').nth(3).getByRole('gridcell').nth(1)).toHaveText('8')
})
