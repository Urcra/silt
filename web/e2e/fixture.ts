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
  teardown(SILT, work, [siltHome])
  rmSync(FILE, { force: true })
}

/**
 * Stops the daemons of `siltHomes` and the processes whose pids are in `pidFiles`, then removes `work`. Each
 * step runs even when one before it fails, as a test may have stopped a daemon already: otherwise a server, its
 * daemons and their data outlive the run, a gigabyte or more each.
 */
export function teardown(silt: string, work: string, siltHomes: string[], pidFiles: string[] = []): void {
  for (const home of siltHomes) {
    try {
      execFileSync(silt, ['daemon', 'stop'], { env: { ...process.env, SILT_HOME: home }, stdio: 'ignore' })
    } catch {
      // Not running.
    }
  }
  for (const pidFile of pidFiles) {
    try {
      process.kill(Number(readFileSync(pidFile, 'utf8')))
    } catch {
      // Gone already.
    }
  }
  // A daemon's agents, or a server just killed, can still be writing there for a moment.
  rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 })
}
