import { execFileSync } from 'node:child_process'
import { readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { type Page, expect, test } from '@playwright/test'

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
  rmSync(live.work, { recursive: true, force: true })
})

async function openFile(page: Page, url: string) {
  await page.goto(url)
  await page.getByRole('link', { name: live.thread }).click()
  await page.getByRole('button', { name: 'src/main.rs' }).click()
  await expect(page.getByTestId('editor')).toContainText('fn farewell() {}')
}

test('two people edit one thread live, see each other and their drafts', async ({ browser }) => {
  const alice = await (await browser.newContext()).newPage()
  const bob = await (await browser.newContext()).newPage()
  await openFile(alice, live.alice)
  await openFile(bob, live.bob)

  await expect(alice.getByTestId('presence')).toContainText('bob is here viewing src/main.rs')
  await expect(bob.getByTestId('presence')).toContainText('alice is here viewing src/main.rs')

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
  await expect(alice.getByTestId('presence')).toContainText('and drafting: now add tests for bob')
  await alice.context().close()
  await bob.context().close()
})

test('following a co-viewer shows their file and line', async ({ browser }) => {
  const alice = await (await browser.newContext()).newPage()
  const bob = await (await browser.newContext()).newPage()
  await alice.goto(live.alice)
  await alice.getByRole('link', { name: live.thread }).click()
  await openFile(bob, live.bob)

  const code = bob.getByTestId('editor').locator('.cm-content')
  await code.click()
  await bob.keyboard.press('ControlOrMeta+Home')
  await bob.keyboard.press('ArrowDown')
  await bob.keyboard.press('ArrowDown')
  await expect(alice.getByTestId('presence')).toContainText('bob is here viewing src/main.rs at line 3')

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
  await openFile(alice, live.alice)
  await openFile(bob, live.bob)
  await bob.getByLabel('draft').fill('mind the tests')
  await expect(alice.getByTestId('presence')).toContainText('drafting: mind the tests')

  await alice.getByLabel('permissions').selectOption('ask_for_everything')
  await alice.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await alice.getByRole('button', { name: 'Start agent' }).click()
  await expect(alice.getByTestId('agent-state')).toHaveText('Fake agent is ready')
  await expect(bob.getByTestId('agent-state')).toHaveText("alice's Fake agent is ready")

  await alice.getByLabel('draft').fill('add acp')
  await alice.getByRole('button', { name: 'Send' }).click()
  const asked = alice.getByRole('group', { name: 'permission' })
  await expect(asked).toContainText('Edit src/main.rs')
  await expect(alice.getByTestId('agent-state')).toHaveText('Fake agent is waiting for your permission')
  await expect(alice.getByRole('region', { name: 'needs you' })).toContainText('needs you')
  await asked.getByRole('button', { name: 'Allow' }).click()

  await expect(alice.getByTestId('agent-state')).toHaveText('Fake agent is ready')
  const reply = alice.getByText(/^Done: You are continuing a conversation/)
  await expect(reply).toContainText('add acp')
  await expect(reply).toContainText('bob: mind the tests')
  await expect(bob.getByText('› add acp')).toBeVisible()
  await expect(bob.getByText('bob: mind the tests').first()).toBeVisible()
  await expect(bob.getByLabel('draft')).toHaveValue('')
  await expect(bob.getByTestId('editor')).toContainText('fn acp() {}', { timeout: 15_000 })

  await alice.getByRole('button', { name: 'Stop agent' }).click()
  await expect(alice.getByRole('button', { name: 'Start agent' })).toBeVisible()
  await alice.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await alice.getByRole('button', { name: 'Start agent' }).click()
  await alice.getByLabel('draft').fill('once more')
  await alice.getByRole('button', { name: 'Send' }).click()
  await alice.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Allow' }).click()
  // A new process of this agent can't load its earlier session, so it starts
  // a new one primed with the conversation.
  await expect(alice.getByText(/^Done: You are continuing a conversation[\s\S]*once more$/)).toBeVisible()
  await alice.getByLabel('draft').fill('not this')
  await alice.getByRole('button', { name: 'Send' }).click()
  await alice.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Reject' }).click()
  await expect(alice.getByText('I was not allowed to edit.')).toBeVisible()

  const changed = await alice.evaluate(async (thread) => {
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
  expect(changed).toContain('+fn acp() {}')
  await alice.context().close()
  await bob.context().close()
})

async function startInNewThread(page: Page, agent: string) {
  await page.goto(live.alice)
  await page.getByRole('main').getByRole('link', { name: 'New thread' }).click()
  await page.getByLabel('agent to start').selectOption({ label: agent })
  await page.getByRole('button', { name: 'Start' }).click()
  await expect(page.getByTestId('agent-state')).toHaveText(`${agent} is ready`)
}

test('the server-hosted app shows an agent waiting on the machine running it', async ({ browser }) => {
  const machine = await (await browser.newContext()).newPage()
  await startInNewThread(machine, 'Fake agent')
  await machine.getByLabel('permissions').selectOption('ask_for_everything')
  await machine.getByLabel('draft').fill('wait for me')
  await machine.getByRole('button', { name: 'Send' }).click()
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
  await expect(waiting.getByText('needs you')).toBeVisible()

  // The browser has no daemon: what it asks goes through the server to the machine.
  await waiting.getByRole('link').first().click()
  const there = hosted.getByRole('region', { name: 'live' })
  await expect(there.getByTestId('agent-state')).toHaveText('Fake agent is waiting for your permission')
  await there.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Allow' }).click()
  await expect(machine.getByTestId('agent-state')).toHaveText('Fake agent is ready')
  await expect(machine.getByText(/^Done: wait for me/)).toBeVisible()
  await expect(there.getByTestId('agent-state')).toHaveText('Fake agent is ready')

  await there.getByLabel('draft').fill('from the browser')
  await there.getByRole('button', { name: 'Send' }).click()
  await expect(asked).toContainText('Edit src/main.rs')
  await expect(there.getByRole('group', { name: 'permission' })).toBeVisible()
  await there.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Reject' }).click()
  await expect(machine.getByText('I was not allowed to edit.')).toBeVisible()
  await expect(async () => {
    await hosted.goto(`${live.server}/app/`)
    await expect(waiting).toHaveCount(0, { timeout: 1_000 })
  }).toPass({ timeout: 15_000 })
  await machine.getByRole('button', { name: 'Stop agent' }).click()
  await machine.context().close()
  await hosted.context().close()
})

test('the server-hosted app continues a thread on a chosen device', async ({ browser }) => {
  const machine = await (await browser.newContext()).newPage()
  await startInNewThread(machine, 'Fake agent')
  await machine.getByLabel('draft').fill('first turn')
  await machine.getByRole('button', { name: 'Send' }).click()
  await expect(machine.getByText('Done: first turn')).toBeVisible()
  await machine.getByRole('button', { name: 'Stop agent' }).click()
  await expect(machine.getByRole('button', { name: 'Start agent' })).toBeVisible()
  const thread = new URL(machine.url()).pathname.split('/t/')[1]

  const hosted = await (await browser.newContext()).newPage()
  await hosted.goto(`${live.server}/app/t/${thread}`)
  await hosted.getByLabel('User').fill('alice')
  await hosted.getByLabel('Password').fill('correct horse battery')
  await hosted.getByRole('button', { name: 'Sign in' }).click()
  const there = hosted.getByRole('region', { name: 'live' })
  await there.getByLabel('device to continue on').selectOption({ label: 'alice' })
  await there.getByLabel('agent to continue with').selectOption({ label: 'Fake agent' })
  await there.getByRole('button', { name: 'Continue there' }).click()
  await expect(machine.getByTestId('agent-state')).toHaveText('Fake agent is ready')
  await expect(there.getByTestId('agent-state')).toHaveText('Fake agent is ready')

  await there.getByLabel('draft').fill('second turn')
  await there.getByRole('button', { name: 'Send' }).click()
  await expect(machine.getByText(/^Done: [\s\S]*second turn$/)).toBeVisible()
  await there.getByRole('button', { name: 'Stop agent' }).click()
  await expect(machine.getByRole('button', { name: 'Start agent' })).toBeVisible()
  await machine.context().close()
  await hosted.context().close()
})

test("another of Alice's machines directs the agent running on the first, through the server", async ({ browser }) => {
  const machine = await (await browser.newContext()).newPage()
  await startInNewThread(machine, 'Fake agent')
  await machine.getByLabel('permissions').selectOption('ask_for_everything')
  await machine.getByLabel('draft').fill('from the desk')
  await machine.getByRole('button', { name: 'Send' }).click()
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
  await laptop.getByRole('button', { name: 'Send' }).click()
  await expect(asked).toContainText('Edit src/main.rs')
  await expect(laptop.getByRole('group', { name: 'permission' })).toBeVisible({ timeout: 10_000 })
  await laptop.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Reject' }).click()
  await expect(machine.getByText('I was not allowed to edit.').last()).toBeVisible()
  await laptop.getByRole('button', { name: 'Stop agent' }).click()
  await expect(machine.getByRole('button', { name: 'Start agent' })).toBeVisible()
  await machine.context().close()
  await laptop.context().close()
})

test('the inbox hears that an agent waits, and answers it in place', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('permissions').selectOption('ask_for_everything')
  await page.getByLabel('draft').fill('ask me in the inbox')
  await page.getByRole('button', { name: 'Send' }).click()
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
  await page.getByRole('button', { name: 'Stop agent' }).click()
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
  await machine.getByRole('button', { name: 'Send' }).click()
  const asked = machine.getByRole('group', { name: 'permission' })
  await expect(asked).toContainText('Edit src/main.rs')
  await expect
    .poll(() => watcher.evaluate(() => (window as unknown as { shown: string[] }).shown.join('\n')), { timeout: 40_000 })
    .toContain('An agent asks for a permission: Fake agent on alice asks to run: Edit src/main.rs')
  await asked.getByRole('button', { name: 'Allow' }).click()
  await machine.getByRole('button', { name: 'Stop agent' }).click()
  await context.close()
  await machine.context().close()
})

test('on a phone, the hosted app answers a waiting agent from Needs you', async ({ browser }) => {
  const machine = await (await browser.newContext()).newPage()
  await startInNewThread(machine, 'Fake agent')
  await machine.getByLabel('permissions').selectOption('ask_for_everything')
  await machine.getByLabel('draft').fill('from my phone')
  await machine.getByRole('button', { name: 'Send' }).click()
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
  await machine.getByRole('button', { name: 'Stop agent' }).click()
  await machine.context().close()
  await phone.context().close()
})

test('a comment written in the server-hosted app reaches the machines', async ({ browser }) => {
  const hosted = await (await browser.newContext()).newPage()
  await hosted.goto(`${live.server}/app/t/${live.thread}`)
  await hosted.getByLabel('User').fill('bob')
  await hosted.getByLabel('Password').fill('correct horse battery')
  await hosted.getByRole('button', { name: 'Sign in' }).click()
  await hosted.getByText('› Please add a farewell line to main').hover()
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
  await page.getByRole('banner').getByRole('link', { name: 'Devices' }).click()
  const here = page.getByRole('region').filter({ has: page.getByText('this device', { exact: true }) })
  await expect(here.getByText('online', { exact: true })).toBeVisible()
  await expect(page.getByRole('region', { name: 'alice-laptop' }).getByText('online', { exact: true })).toBeVisible()
  const prompts = here.getByLabel('Send prompts')
  await expect(prompts).toBeChecked()
  await prompts.click()
  await expect(prompts).not.toBeChecked()
  await page.reload()
  await expect(prompts).not.toBeChecked()
  await prompts.click()
  await expect(prompts).toBeChecked()
})

test('a new thread starts with an agent in a known clone, which edits without asking by default', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await expect(page.getByLabel('permissions')).toHaveValue('ask_before_commands')
  await page.getByLabel('draft').fill('hello')
  await page.getByRole('button', { name: 'Send' }).click()
  await expect(page.getByText('Done: hello')).toBeVisible()
  await expect(page.getByRole('group', { name: 'permission' })).toHaveCount(0)
})

test('prompts queue while the agent works, and a steer interrupts its turn', async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('permissions').selectOption('ask_for_everything')
  const draft = page.getByLabel('draft')
  const permission = page.getByRole('group', { name: 'permission' })

  await draft.fill('first')
  await page.getByRole('button', { name: 'Send' }).click()
  await expect(permission).toBeVisible()
  await draft.fill('second')
  await page.getByRole('button', { name: 'Queue', exact: true }).click()
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
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'New thread' }).click()
  const form = page.getByRole('form', { name: 'new thread' })
  await form.getByLabel('first prompt').fill('write the docs')
  await form.getByLabel('base branch').selectOption('origin/main')
  await form.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await form.getByLabel('permissions').selectOption('ask_for_everything')
  await form.getByLabel('visibility').selectOption('private')
  await form.getByLabel('Plan first').check()
  await form.getByLabel('Start in the background').check()
  await form.getByRole('button', { name: 'Start' }).click()

  await expect(page.getByText('Fake agent started in a new thread')).toBeVisible()
  await expect(form.getByLabel('first prompt')).toHaveValue('')
  const waiting = page.getByRole('region', { name: 'needs you' })
  await waiting.getByRole('link').first().click()
  await expect(page.getByText('Plan: write the docs')).toBeVisible()
  await expect(page.getByTestId('agent-state')).toHaveText('Fake agent wrote a plan for you to approve')
  await expect(page.getByLabel('permissions')).toHaveValue('ask_for_everything')
})

test("the agent's modes and model are offered, and Plan first plans in its plan mode", async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  const mode = page.getByLabel('mode', { exact: true })
  await expect(mode).toHaveValue('code')
  await page.getByLabel('model').selectOption({ label: 'Smart' })
  await expect(page.getByLabel('model')).toHaveValue('smart')

  await page.getByLabel('draft').fill('add a parser')
  await page.getByLabel('Plan first').check()
  await page.getByRole('button', { name: 'Send' }).click()
  await expect(page.getByText('Plan: add a parser')).toBeVisible()
  await expect(page.getByTestId('agent-state')).toHaveText('Fake agent wrote a plan for you to approve')
  await expect(mode).toHaveValue('plan')
  await expect(page.getByRole('region', { name: 'needs you' })).toBeVisible()

  await page.getByRole('group', { name: 'plan' }).getByRole('button', { name: 'Approve plan' }).click()
  await expect(page.getByText('Done: The plan is approved. Go ahead.')).toBeVisible()
  await expect(mode).toHaveValue('code')
  await expect(page.getByTestId('agent-state')).toHaveText('Fake agent is ready')
})

test('Plan first asks an agent without a plan mode for a plan by the prompt', async ({ page }) => {
  await startInNewThread(page, 'Plain agent')
  await expect(page.getByLabel('mode', { exact: true })).toHaveCount(0)
  await page.getByLabel('draft').fill('add a lexer')
  await page.getByLabel('Plan first').check()
  await page.getByRole('button', { name: 'Send' }).click()
  await expect(page.getByText(/^Done: Before you change anything, write a plan[\s\S]*add a lexer$/)).toBeVisible()
  await expect(page.getByTestId('agent-state')).toHaveText('Plain agent wrote a plan for you to approve')

  await page.getByLabel('draft').fill('use a table instead')
  await page.getByRole('button', { name: 'Send' }).click()
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
  await page.getByRole('button', { name: 'Send' }).click()
  await expect(page.getByText('Done: look at these (with an image (image/png), attachment:notes.txt saying "hello")')).toBeVisible()
  await expect(page.getByText(/\(attached: prompt\/\w+\/shot\.png, prompt\/\w+\/notes\.txt\)/)).toBeVisible()
  await expect(page.getByRole('list', { name: 'attached files' })).toBeHidden()
})

test("an agent that takes no images is told a text file's content in the prompt, and an image is refused", async ({ page }) => {
  await startInNewThread(page, 'Plain agent')
  await page.getByLabel('attach files').setInputFiles([SHOT])
  await page.getByLabel('draft').fill('see this')
  await page.getByRole('button', { name: 'Send' }).click()
  await expect(page.getByText("Plain agent doesn't take files like shot.png")).toBeVisible()

  await page.getByRole('button', { name: 'remove shot.png' }).click()
  await page.getByLabel('attach files').setInputFiles([NOTES])
  await page.getByRole('button', { name: 'Send' }).click()
  await expect(page.getByText(/^Done: see this\s+notes\.txt:\s+```\s+hello\s+```$/)).toBeVisible()
})

test("comments on the agent's last turn go back to it as a prompt", async ({ page }) => {
  await startInNewThread(page, 'Fake agent')
  await page.getByLabel('draft').fill('add a function')
  await page.getByRole('button', { name: 'Send' }).click()
  await expect(page.getByText('Done: add a function')).toBeVisible()

  await page.getByRole('button', { name: 'Show changes' }).click()
  const changes = page.getByRole('region', { name: 'changes' })
  await page.getByRole('tab', { name: 'Last turn' }).click()
  await expect(changes.getByTestId('diff')).toContainText('+fn acp() {}')
  await changes.getByRole('button', { name: /^comment on src\/main\.rs line \d+$/ }).last().click()
  await page.getByLabel('code comment').fill('give it a better name')
  await page.getByRole('button', { name: 'Add', exact: true }).click()
  await expect(changes.getByText('alice: give it a better name')).toBeVisible()

  await page.getByRole('button', { name: 'Send 1 comment to the agent' }).click()
  await expect(page.getByText('Sent to the agent')).toBeVisible()
  await expect(
    page.getByText(/^Done: Please address these review comments:\s+- src\/main\.rs:\d+ \(alice\): give it a better name$/),
  ).toBeVisible()

  await page.getByRole('tab', { name: 'Against main' }).click()
  await expect(changes.getByTestId('diff')).toContainText('+fn acp() {}')
})

/** Starts the fake agent in a new thread of the project whose name matches, asking before everything. */
async function startInBackground(page: Page, project: RegExp, prompt: string) {
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'New thread' }).click()
  const form = page.getByRole('form', { name: 'new thread' })
  const projects = form.getByLabel('project')
  await projects.selectOption((await projects.locator('option').filter({ hasText: project }).getAttribute('value')) ?? '')
  await form.getByLabel('first prompt').fill(prompt)
  await form.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await form.getByLabel('permissions').selectOption('ask_for_everything')
  await form.getByLabel('Start in the background').check()
  await form.getByRole('button', { name: 'Start' }).click()
  await expect(form.getByLabel('first prompt')).toHaveValue('')
}

test('two agents in two projects queue prompts, are answered from the sidebar, and a fork is offered back', async ({ page }) => {
  await page.goto(live.alice)
  await startInBackground(page, /origin/, 'work in origin')
  await startInBackground(page, /tools/, 'work in tools')

  const sidebar = page.getByRole('navigation', { name: 'projects' })
  const origin = sidebar.getByRole('group', { name: 'work in origin needs you' })
  const tools = sidebar.getByRole('group', { name: 'work in tools needs you' })
  await expect(origin).toContainText('Asks to run: Edit src/main.rs')
  await expect(tools).toContainText('Asks to run: Edit src/main.rs')

  await origin.getByRole('link', { name: 'work in origin' }).click()
  await page.getByLabel('draft').fill('then document it')
  await page.getByRole('button', { name: 'Queue', exact: true }).click()
  await expect(page.getByRole('region', { name: 'queued prompts' })).toContainText('then document it')

  await tools.getByRole('button', { name: 'Allow' }).click()
  await expect(tools).toBeHidden()
  await origin.getByRole('button', { name: 'Allow' }).click()
  await expect(page.getByText('Done: work in origin')).toBeVisible()
  // The queued prompt went next, and asks in turn.
  await expect(origin).toContainText('Asks to run: Edit src/main.rs')
  await origin.getByRole('button', { name: 'Allow' }).click()
  await expect(page.getByText('Done: then document it')).toBeVisible()
  await expect(origin).toBeHidden()

  await page.getByRole('button', { name: 'Fork', exact: true }).click()
  await expect(page.getByRole('main').getByText(/^Fork of/)).toBeVisible()
  await page.getByRole('button', { name: 'Offer as merge request' }).click()
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
  await page.getByRole('main').getByRole('link').filter({ hasText: /^[0-9a-f]{12}$/ }).first().click()
  await page.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await page.getByRole('button', { name: 'Start agent' }).click()
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
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'Projects', exact: true }).click()
  const list = page.getByTestId('projects')
  await expect(list.getByRole('listitem', { name: 'tools' })).toContainText(`Here at ${live.work}/alice/tools`)
  const origin = list.getByRole('listitem', { name: 'origin' })
  await expect(origin).toContainText(/\d+ threads?/)
  await expect(origin).toContainText(`Here at ${live.work}/alice/repo`)
  await expect(origin).toContainText('New threads start with plain')

  const add = page.getByRole('region', { name: 'add a project' })
  await add.getByLabel('clone path').fill(live.work)
  await add.getByRole('button', { name: 'Add' }).click()
  await expect(add).toContainText('not inside a git repository')

  await add.getByRole('tab', { name: 'Clone a URL' }).click()
  await add.getByLabel('remote URL').fill(`${live.work}/docs.git`)
  await add.getByLabel('where to clone it').fill(`${live.work}/alice/docs`)
  await add.getByRole('button', { name: 'Add' }).click()
  await expect(list.getByRole('listitem', { name: 'docs' })).toContainText(`Here at ${live.work}/alice/docs`)
  expect(readFileSync(join(live.work, 'alice', 'docs', 'src', 'main.rs'), 'utf8')).toBe('fn tool() {}\n')
})

test("a new thread starts with what its project says, until the person picks", async ({ page }) => {
  await page.goto(live.alice)
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'New thread' }).click()
  const form = page.getByRole('form', { name: 'new thread' })
  const projects = form.getByLabel('project')
  const pick = async (project: RegExp) =>
    projects.selectOption((await projects.locator('option').filter({ hasText: project }).getAttribute('value')) ?? '')
  await pick(/origin/)
  await expect(form.getByLabel('agent to start')).toHaveValue('plain')
  await pick(/tools/)
  await expect(form.getByLabel('agent to start')).toHaveValue('')
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
  const shared = (page: Page) => sidebar(page).getByRole('link', { name: /Add a farewell$/ }).first()
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
  const snoozed = sidebar(alice).getByRole('region', { name: 'snoozed' })
  await expect(snoozed).toContainText('1')
  await snoozed.getByRole('button', { expanded: false }).click()
  await snoozed.getByRole('button', { name: 'actions for Add a farewell' }).click()
  await alice.getByRole('menuitem', { name: 'Wake up' }).click()
  await expect(snoozed).toHaveCount(0)

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
  await expect(sidebar(alice).getByRole('region', { name: 'settled' })).toContainText('1')
  await alice.context().close()
  await bob.context().close()
})

test("an org's owners set it up in the web UI, and its members only see it", async ({ browser }) => {
  const data = join(live.work, 'server')
  execFileSync(SILT, ['server', 'user', 'admin', 'alice', '--data', data])
  const alice = await (await browser.newContext()).newPage()
  await alice.goto(live.alice)
  await alice.getByRole('link', { name: 'Orgs' }).click()
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
  await bob.getByRole('link', { name: 'Orgs' }).click()
  const seen = bob.getByRole('region', { name: 'acme', exact: true })
  await expect(seen).toContainText('member')
  await expect(seen.getByLabel('default visibility of acme')).toBeDisabled()
  await expect(seen.getByRole('button', { name: 'Add member' })).toHaveCount(0)
  await alice.context().close()
  await bob.context().close()
})

test('the owner shares a thread from its page: who sees it, and who may write into it', async ({ page }) => {
  await page.goto(live.alice)
  await page.getByRole('link', { name: live.thread }).click()
  await page.getByRole('button', { name: 'Share' }).click()
  const dialog = page.getByRole('dialog', { name: 'Share this thread' })
  const collaborators = dialog.getByRole('list', { name: 'collaborators' })
  await expect(collaborators).toContainText('bob')

  await collaborators.getByRole('button', { name: 'Remove' }).click()
  await expect(collaborators).toContainText('Nobody yet.')
  await dialog.getByLabel('person to add').fill('bob')
  await dialog.getByRole('button', { name: 'Add', exact: true }).click()
  await expect(collaborators).toContainText('bob')

  await dialog.getByLabel('who sees it').selectOption('private')
  await expect(dialog.getByLabel('who sees it')).toHaveValue('private')
  await dialog.getByLabel('who sees it').selectOption('org')
  await expect(dialog.getByLabel('who sees it')).toHaveValue('org')

  await dialog.getByLabel('person to add').fill('nobody-by-that-name')
  await dialog.getByRole('button', { name: 'Add', exact: true }).click()
  await expect(dialog).toContainText('could not share the thread')
})
