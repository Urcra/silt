import { execFileSync } from 'node:child_process'
import { readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test } from '@playwright/test'
import { stopAgent } from './fixture'

const SILT = join(import.meta.dirname, '..', '..', 'target', 'debug', 'silt')

interface Fixture {
  url: string
  dir: string
}

let fixture: Fixture

/** The fixture machine's environment, for running `silt` as it does. */
const machine = () => ({
  ...process.env,
  SILT_HOME: join(fixture.dir, 'me', 'silt'),
  HOME: join(fixture.dir, 'me', 'home'),
  CLAUDE_CONFIG_DIR: join(fixture.dir, 'me', 'claude'),
})

test.beforeAll(() => {
  const output = execFileSync(join(import.meta.dirname, 'local-fixture.sh'), {
    encoding: 'utf8',
  })
  fixture = JSON.parse(output.trim().split('\n').pop() ?? '{}') as Fixture
})

test.afterAll(() => {
  execFileSync(SILT, ['daemon', 'stop'], { env: machine() })
  process.kill(Number(readFileSync(join(fixture.dir, 'hub.pid'), 'utf8')))
  rmSync(fixture.dir, {
    recursive: true,
    force: true,
    maxRetries: 10,
    retryDelay: 100,
  })
})

test('a local model answers in a Local only workspace, and its thread can be erased (ADR 0070)', async ({ page }) => {
  await page.goto(fixture.url)
  await page.goto(new URL('/settings/models', fixture.url).href)
  const models = page.getByRole('region', { name: 'Local models' })
  await expect(models.getByRole('list', { name: 'models here' })).toContainText('Tiny')
  await expect(models).toContainText('Fake GPU')

  // A model is found on Hugging Face, chosen by its quantization, and downloaded.
  const found = models.getByRole('list', { name: 'found on Hugging Face' })
  await found.getByRole('button', { name: /owner\/tiny-GGUF/ }).click()
  const quantizations = found.getByRole('list', { name: 'quantizations of owner/tiny-GGUF' })
  await expect(quantizations).toContainText('Q4_K_M')
  await expect(quantizations).toContainText('Fits')
  await quantizations.getByRole('button', { name: 'Download' }).click()
  await expect(models.getByRole('list', { name: 'models here' })).toContainText('tiny-q4_k_m')

  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'New thread', exact: true }).click()
  const form = page.getByRole('form', { name: 'new thread' })
  await form.getByLabel('agent to start').selectOption({ label: 'Silt (local model)' })
  await form.getByRole('radio', { name: 'Local' }).click()
  await form.getByLabel('first prompt').fill('What failed in app.log?')
  await form.getByLabel('first prompt').press('Enter')
  await expect(page.getByText('Heard: What failed in app.log?')).toBeVisible()
  await stopAgent(page)

  await page.getByRole('button', { name: 'More for this thread' }).click()
  await page.getByRole('button', { name: 'Erase from this machine…' }).click()
  const erase = page.getByRole('alertdialog', { name: 'erase this thread' })
  await expect(erase).toContainText('removes thread')
  await erase.getByRole('button', { name: 'Erase for good' }).click()
  await expect(page.getByText(/^erased /)).toBeVisible()
  const threads = execFileSync(SILT, ['log'], {
    env: machine(),
    encoding: 'utf8',
  })
  expect(threads).not.toContain('What failed')
})
