import { execFileSync } from 'node:child_process'
import { writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { type Page, expect, test } from '@playwright/test'
import { stopAgent, teardown } from './fixture'

const SILT = join(import.meta.dirname, '..', '..', 'target', 'debug', 'silt')

interface Fixture {
  ada: string
  work: string
  oss: string
  dir: string
}

let fixture: Fixture

test.beforeAll(() => {
  const output = execFileSync(join(import.meta.dirname, 'workspaces-fixture.sh'), { encoding: 'utf8' })
  fixture = JSON.parse(output.trim().split('\n').pop() ?? '{}') as Fixture
})

test.afterAll(() => {
  const servers = ['work', 'oss'].map((server) => join(fixture.dir, `${server}.pid`))
  teardown(SILT, fixture.dir, [join(fixture.dir, 'ada', 'silt')], servers)
})

const host = (url: string) => new URL(url).host

/** Starts the fake agent in a new thread of the project whose name matches, then stops it. */
async function startIn(page: Page, project: RegExp, prompt: string) {
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'New thread', exact: true }).click()
  const form = page.getByRole('form', { name: 'new thread' })
  await page.getByRole('button', { name: /^project:/ }).click()
  await page.getByRole('dialog', { name: 'Pick a project' }).getByRole('option', { name: project }).click()
  await form.getByLabel('first prompt').fill(prompt)
  await form.getByRole('radio', { name: 'Worktree' }).click()
  await form.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await form.getByLabel('first prompt').press('Enter')
  await expect(page.getByText(new RegExp(`^Done: [\\s\\S]*${prompt}$`))).toBeVisible()
  await stopAgent(page)
}

test('each server is a workspace, and the sidebar shows only the threads of the one chosen', async ({ page }) => {
  await page.goto(fixture.ada)
  const switcher = page.getByRole('button', { name: /^workspace: / })
  const choose = async (name: string | RegExp) => {
    await switcher.click()
    await page.getByRole('dialog', { name: 'Switch workspace' }).getByRole('button', { name }).click()
  }
  await switcher.click()
  const workspaces = page.getByRole('dialog', { name: 'Switch workspace' })
  await expect(workspaces.getByRole('region', { name: /^On / })).toHaveCount(2)
  await expect(workspaces.getByRole('region', { name: /^On / }).first()).toHaveAccessibleName(`On ${host(fixture.work)}`)
  await page.keyboard.press('Escape')

  // The tools project's threads go to the open-source server; the app's to work, the default.
  await page.getByRole('link', { name: 'Settings', exact: true }).click()
  const tools = page.getByTestId('projects').getByRole('listitem', { name: 'tools' }).first()
  await tools.getByLabel('where tools syncs').selectOption(fixture.oss)
  await expect(page.getByText(/new threads of .* sync with/).first()).toBeVisible()
  await startIn(page, /app/, 'work on the app')
  await startIn(page, /tools/, 'work on the tools')

  const sidebar = page.getByRole('navigation', { name: 'projects' })
  await expect(sidebar).toContainText('work on the app')
  await expect(sidebar).toContainText('work on the tools')
  await choose(new RegExp(`^All of ${host(fixture.oss)}`))
  await expect(sidebar).toContainText('work on the tools')
  await expect(sidebar).not.toContainText('work on the app')
  await choose(new RegExp(`^All of ${host(fixture.work)}`))
  await expect(sidebar).toContainText('work on the app')
  await expect(sidebar).not.toContainText('work on the tools')
  await choose(/^This machine/)
  await expect(sidebar).not.toContainText('work on the')

  // The choice holds for the tab, across a reload.
  await page.reload()
  await expect(switcher).toHaveAccessibleName('workspace: this machine')
  await choose(/^Everything/)
  await expect(sidebar).toContainText('work on the tools')

  await switcher.click()
  await page.getByRole('button', { name: 'Connect another server' }).click()
  await expect(page.getByRole('dialog', { name: 'Connect another server' })).toContainText('silt auth login --server')
})

/** `silt` as Ada's machine runs it, in her clone of `project`. */
function ada(args: string[], project = 'app'): string {
  const home = join(fixture.dir, 'ada')
  return execFileSync(SILT, args, {
    cwd: join(home, project),
    encoding: 'utf8',
    env: { ...process.env, SILT_HOME: join(home, 'silt'), HOME: join(home, 'home'), CLAUDE_CONFIG_DIR: join(home, 'claude') },
  })
}

/** A screenshot for the pull request and the guide, when `SILT_SCREENSHOTS_DIR` asks for them. */
async function shoot(page: Page, name: string, height = 720) {
  const dir = process.env.SILT_SCREENSHOTS_DIR
  if (!dir) return
  await page.setViewportSize({ width: 1280, height })
  await page.screenshot({ path: join(dir, `${name}.png`) })
  await page.setViewportSize({ width: 1280, height: 720 })
}

test("workspaces keep work and personal apart, with the org's rules on work (ADR 0065)", async ({ page }) => {
  // acme on the work server, whose policy caps what agents do and keeps its projects there.
  execFileSync(SILT, ['server', 'user', 'admin', 'ada', '--data', join(fixture.dir, 'work')])
  ada(['org', 'create', 'acme'])
  ada(['org', 'policy', 'acme', '--agents', 'copilot', '--max-preset', 'ask_before_commands', '--home-locked'])
  ada(['workspace', 'new', 'Work', '--server', fixture.work, '--org', 'acme', '--folder', join(fixture.dir, 'ada', 'app')])
  ada(['workspace', 'new', 'Personal', '--server', fixture.oss, '--folder', join(fixture.dir, 'ada', 'tools'), '--default'])
  ada(['workspace', 'remove', 'default'])
  ada(['workspace', 'policy', 'Work', '--hide-other-workspaces', 'on'])
  ada(['workspace', 'env', 'Work', `CLAUDE_CONFIG_DIR=${join(fixture.dir, 'ada', 'work-claude')}`])
  await expect.poll(() => ada(['workspace', 'list']), { timeout: 15_000 }).toContain("acme's policy")
  expect(ada(['workspace', 'which'])).toContain('Work (by rule folder')

  await page.goto(fixture.ada)
  const switcher = page.getByRole('button', { name: /^workspace: / })
  await switcher.click()
  const workspaces = page.getByRole('dialog', { name: 'Switch workspace' })
  await expect(workspaces.getByRole('region', { name: 'Workspaces' }).getByRole('button')).toHaveCount(2)
  await expect(workspaces.getByRole('button', { name: /^Work/ })).toContainText("acme's rules")
  await shoot(page, '1-switcher')
  await workspaces.getByRole('button', { name: /^Work/ }).click()
  const sidebar = page.getByRole('navigation', { name: 'projects' })
  await expect(sidebar).toContainText('work on the app')
  await expect(sidebar).not.toContainText('work on the tools')

  await page.goto(`${new URL(fixture.ada).origin}/settings/workspaces${new URL(fixture.ada).hash}`)
  const settings = page.getByRole('region', { name: 'Workspaces' })
  await settings.getByRole('tab', { name: /Work/ }).click()
  await expect(settings.getByRole('region', { name: 'Rules' })).toContainText("acme on")
  await expect(settings.getByRole('region', { name: 'Rules' })).toContainText('Ask before commands')
  await expect(settings.getByLabel('server')).toBeDisabled()
  await shoot(page, '2-settings', 1900)

  await page.getByRole('link', { name: 'Projects', exact: true }).click()
  await expect(page.getByRole('region', { name: 'Work projects' })).toContainText('app')
  await expect(page.getByRole('region', { name: 'Personal projects' })).toContainText('tools')
  await shoot(page, '4-projects')

  await page.getByRole('link', { name: 'Orgs', exact: true }).click()
  await expect(page.getByRole('region', { name: "acme's policy" })).toContainText('GitHub Copilot only · at most Ask before commands · home locked')
  await shoot(page, '6-org-policy', 820)

  await sidebar.getByRole('link', { name: 'New thread', exact: true }).click()
  await page.getByRole('button', { name: /^project:/ }).click()
  await page.getByRole('dialog', { name: 'Pick a project' }).getByRole('option', { name: /app/ }).click()
  const line = page.getByLabel('workspace', { exact: true })
  await expect(line).toContainText('In Work')
  await expect(line).toContainText("GitHub Copilot only · at most Ask before commands · other workspaces' files hidden · home locked")
  await expect(page.getByRole('form', { name: 'new thread' })).toContainText('No agent installed here is allowed in Work')
  await shoot(page, '3-new-thread')

  // The daemon refuses what the page greys out, however it's asked.
  const [daemon, token] = fixture.ada.split('#token=')
  const refused = await page.request.post(`${new URL(daemon!).origin}/api/agents/start`, {
    headers: { authorization: `Bearer ${token}` },
    data: { agent: 'fake', clone: join(fixture.dir, 'ada', 'app'), prompt: 'hi' },
  })
  expect(refused.ok()).toBe(false)
  expect(await refused.text()).toContain("Fake agent isn't allowed in Work: acme's policy on")
})

test("a workspace's defaults pick the agent's model, and the project picker moves to a project by its first letter", async ({ page }) => {
  await page.goto(`${new URL(fixture.ada).origin}/settings/workspaces${new URL(fixture.ada).hash}`)
  const settings = page.getByRole('region', { name: 'Workspaces' })
  await settings.getByRole('tab', { name: /Personal/ }).click()
  await settings.getByLabel('agent whose defaults to set').selectOption({ label: 'Fake agent' })
  await settings.getByLabel('default model for Fake agent').selectOption({ label: 'Smart' })
  await settings.getByRole('button', { name: 'Save workspaces' }).click()
  await expect(page.getByText('Workspaces saved')).toBeVisible()

  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'New thread', exact: true }).click()
  await page.getByRole('button', { name: /^project:/ }).click()
  const picker = page.getByRole('dialog', { name: 'Pick a project' })
  await page.keyboard.press('t')
  await expect(picker.getByRole('option', { name: /tools/ }).getByRole('button')).toBeFocused()
  await page.keyboard.press('ArrowUp')
  await expect(picker.getByRole('option', { name: /app/ }).getByRole('button')).toBeFocused()
  await page.keyboard.press('t')
  await expect(picker.getByRole('option', { name: /tools/ }).getByRole('button')).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page.getByRole('button', { name: 'project: tools' })).toBeVisible()
  const form = page.getByRole('form', { name: 'new thread' })
  await form.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await expect(form.getByLabel('model')).toHaveValue('smart')
})

test("a workspace's sandbox reach and its plugins are set in its settings, and a plugin is on only where turned on (ADR 0093)", async ({ page }) => {
  const file = join(fixture.dir, 'fleet-api.json')
  const environment = { target: 'https://api.fleet.example/graphql', reads: 'allow', changes: 'ask', data: ['location'] }
  writeFileSync(file, JSON.stringify({ name: 'fleet-api', kind: { type: 'graphql' }, environments: { production: environment } }))
  ada(['plugin', 'add', file])
  await page.goto(`${new URL(fixture.ada).origin}/settings/workspaces${new URL(fixture.ada).hash}`)
  const settings = page.getByRole('region', { name: 'Workspaces' })
  await settings.getByRole('tab', { name: /Personal/ }).click()

  const plugins = settings.getByRole('region', { name: 'Plugins its agents can use' })
  await expect(plugins).toContainText('fleet-api')
  await expect(plugins).toContainText('production · location kept')
  await plugins.getByLabel('fleet-api in Personal').click()
  await expect(plugins.getByLabel('fleet-api in Personal')).toBeChecked()
  await expect.poll(() => ada(['plugin', 'list'])).toContain('on in personal')
  await settings.getByRole('tab', { name: /Work/ }).click()
  await expect(plugins.getByLabel('fleet-api in Work')).not.toBeChecked()

  const reach = settings.getByRole('region', { name: 'What its agents can reach' })
  await expect(reach.getByRole('radio', { name: /Full internet/ })).toBeChecked()
  await reach.getByRole('radio', { name: /No network/ }).check()
  await reach.getByLabel('The GPU').check()
  await settings.getByRole('button', { name: 'Save workspaces' }).click()
  await expect(page.getByText('Workspaces saved')).toBeVisible()
  await page.reload()
  await settings.getByRole('tab', { name: /Work/ }).click()
  await expect(reach.getByRole('radio', { name: /No network/ })).toBeChecked()
  await expect(reach.getByLabel('The GPU')).toBeChecked()
  await reach.scrollIntoViewIfNeeded()
  await shoot(page, '7-sandbox-settings', 1600)

  // A thread's header says what its agent's sandbox reaches, to read rather than change.
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: /work on the app/ }).first().click()
  await page.getByRole('button', { name: 'Sandbox' }).click()
  const shield = page.getByRole('dialog', { name: 'Sandbox' })
  await expect(shield).toContainText('The next run is sandboxed')
  await expect(shield).toContainText('None')
  await expect(shield).toContainText('the GPU')
  await shoot(page, '8-sandbox-shield')
})
