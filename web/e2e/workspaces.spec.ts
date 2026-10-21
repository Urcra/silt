import { execFileSync } from 'node:child_process'
import { readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { type Page, expect, test } from '@playwright/test'

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
  execFileSync(SILT, ['daemon', 'stop'], { env: { ...process.env, SILT_HOME: join(fixture.dir, 'ada', 'silt') } })
  for (const server of ['work', 'oss']) {
    process.kill(Number(readFileSync(join(fixture.dir, `${server}.pid`), 'utf8')))
  }
  rmSync(fixture.dir, { recursive: true, force: true })
})

const host = (url: string) => new URL(url).host

/** Starts the fake agent in a new thread of the project whose name matches, then stops it. */
async function startIn(page: Page, project: RegExp, prompt: string) {
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'New thread' }).click()
  const form = page.getByRole('form', { name: 'new thread' })
  const projects = form.getByLabel('project')
  await projects.selectOption((await projects.locator('option').filter({ hasText: project }).getAttribute('value')) ?? '')
  await form.getByLabel('first prompt').fill(prompt)
  await form.getByLabel('agent to start').selectOption({ label: 'Fake agent' })
  await form.getByRole('button', { name: 'Start' }).click()
  await expect(page.getByText(new RegExp(`^Done: [\\s\\S]*${prompt}$`))).toBeVisible()
  await page.getByRole('button', { name: 'Stop agent' }).click()
}

test('each server is a workspace, and the sidebar shows only the threads of the one chosen', async ({ page }) => {
  await page.goto(fixture.ada)
  const workspace = page.getByRole('combobox', { name: 'workspace', exact: true })
  await expect(workspace.locator('optgroup')).toHaveCount(2)
  await expect(workspace.locator('optgroup').first()).toHaveAttribute('label', host(fixture.work))

  // The tools project's threads go to the open-source server; the app's to work, the default.
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'Projects', exact: true }).click()
  const tools = page.getByTestId('projects').getByRole('listitem', { name: 'tools' }).first()
  await tools.getByLabel('where tools syncs').selectOption(fixture.oss)
  await expect(page.getByText(/new threads of .* sync with/).first()).toBeVisible()
  await startIn(page, /app/, 'work on the app')
  await startIn(page, /tools/, 'work on the tools')

  const sidebar = page.getByRole('navigation', { name: 'projects' })
  await expect(sidebar).toContainText('work on the app')
  await expect(sidebar).toContainText('work on the tools')
  await workspace.selectOption({ label: `All of ${host(fixture.oss)}` })
  await expect(sidebar).toContainText('work on the tools')
  await expect(sidebar).not.toContainText('work on the app')
  await workspace.selectOption({ label: `All of ${host(fixture.work)}` })
  await expect(sidebar).toContainText('work on the app')
  await expect(sidebar).not.toContainText('work on the tools')
  await workspace.selectOption({ label: 'This machine' })
  await expect(sidebar).not.toContainText('work on the')

  // The choice holds for the tab, across a reload.
  await page.reload()
  await expect(workspace).toHaveValue('local')
  await workspace.selectOption({ label: 'Everything' })
  await expect(sidebar).toContainText('work on the tools')

  await workspace.selectOption({ label: 'Connect another server…' })
  await expect(page.getByRole('dialog', { name: 'Connect another server' })).toContainText('silt auth login --server')
})
