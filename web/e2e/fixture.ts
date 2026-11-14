import { execFileSync } from 'node:child_process'
import { readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import type { Locator, Page } from '@playwright/test'

export interface Fixture {
  url: string
  siltHome: string
  work: string
  thread: string
  fork: string
}

const FILE = join(import.meta.dirname, '.fixture.json')
/** The `silt` the fixture runs: `$SILT`, as the performance budgets are timed
 * on an optimized build, else the debug build. */
export const SILT = process.env.SILT ?? join(import.meta.dirname, '..', '..', 'target', 'debug', 'silt')

export function createFixture(): void {
  const output = execFileSync(join(import.meta.dirname, 'fixture.sh'), { encoding: 'utf8' })
  writeFileSync(FILE, output.trim().split('\n').pop() ?? '')
}

export function fixture(): Fixture {
  return JSON.parse(readFileSync(FILE, 'utf8')) as Fixture
}

/** Runs `silt` against the fixture's data, as a person at a terminal would. */
export function silt(...args: string[]): string {
  const { siltHome } = fixture()
  return execFileSync(SILT, args, { encoding: 'utf8', env: { ...process.env, SILT_HOME: siltHome } })
}

/** Stops the thread's agent, from the composer's agent settings where it is offered. Their panel opens over the
 * whole page, outside a part of it the test may look in. */
export async function stopAgent(on: Page | Locator): Promise<void> {
  await on.getByRole('button', { name: 'agent settings' }).click()
  const page = 'page' in on ? on.page() : on
  await page.getByRole('dialog', { name: 'agent settings' }).getByRole('button', { name: 'Stop agent' }).click()
}

/** Opens All threads from the sidebar: `/` is a new thread (ADR 0044). */
export async function allThreads(page: Page): Promise<void> {
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'All threads' }).click()
}

export function removeFixture(): void {
  const { siltHome, work } = fixture()
  execFileSync(SILT, ['daemon', 'stop'], { env: { ...process.env, SILT_HOME: siltHome } })
  rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 })
  rmSync(FILE, { force: true })
}
