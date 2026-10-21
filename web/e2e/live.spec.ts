import { execFileSync } from 'node:child_process'
import { readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { type Page, expect, test } from '@playwright/test'

const SILT = join(import.meta.dirname, '..', '..', 'target', 'debug', 'silt')

interface Live {
  alice: string
  bob: string
  thread: string
  work: string
}

let live: Live

test.beforeAll(() => {
  const output = execFileSync(join(import.meta.dirname, 'live-fixture.sh'), { encoding: 'utf8' })
  live = JSON.parse(output.trim().split('\n').pop() ?? '{}') as Live
})

test.afterAll(() => {
  for (const user of ['alice', 'bob']) {
    execFileSync(SILT, ['daemon', 'stop'], { env: { ...process.env, SILT_HOME: join(live.work, user, 'silt') } })
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
  await page.getByRole('group', { name: 'permission' }).getByRole('button', { name: 'Allow' }).click()
  await expect(page.getByText('Done: I interrupted your last turn to say: change of plan')).toBeVisible()
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
  await page.getByRole('button', { name: 'Add' }).click()
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
