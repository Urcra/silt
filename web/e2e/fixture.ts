import { execFileSync, spawnSync } from 'node:child_process'
import { appendFileSync, existsSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
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

/**
 * Runs the fixture script `script` in this folder and gives the last line it printed, the world it made. When it
 * fails, the error says why: execFileSync's names only the command, its stderr gone to the run's output, apart.
 */
export function runFixture(script: string, env: NodeJS.ProcessEnv = process.env): string {
  const ran = spawnSync(join(import.meta.dirname, script), { encoding: 'utf8', env, maxBuffer: 64 << 20 })
  if (ran.error) throw ran.error
  if (ran.status !== 0) {
    const said = ran.stderr.trim().split('\n').slice(-40).join('\n')
    throw new Error(`${script} failed (${ran.signal ?? `exit ${ran.status}`}):\n${said}`)
  }
  const line = ran.stdout.trim().split('\n').pop() ?? ''
  try {
    const { work, dir } = JSON.parse(line) as { work?: unknown; dir?: unknown }
    const folder = work ?? dir
    if (typeof folder === 'string') appendFileSync(MADE, `${folder}\n`)
  } catch {
    // Not a world, which the spec says when it reads it.
  }
  return line
}

/** The folders the fixtures made in this run, so what a spec didn't tear down is ended anyway. */
const MADE = join(import.meta.dirname, '.fixtures-made')

/**
 * Stops the daemons and servers the fixtures left running and removes their folders. A worker whose test ran out
 * of time can be ended before its `afterAll` tears its fixture down, leaving a server, its daemons and a gigabyte
 * or more behind; this runs before and after the whole run.
 */
export function abandonFixtures(): void {
  let made: string[] = []
  try {
    made = readFileSync(MADE, 'utf8').split('\n').filter(Boolean)
  } catch {
    return
  }
  for (const folder of made.filter((folder) => existsSync(folder)))
    spawnSync(join(import.meta.dirname, 'abandon.sh'), [folder, SILT], { stdio: 'ignore' })
  rmSync(MADE, { force: true })
}

export function createFixture(): void {
  writeFileSync(FILE, runFixture('fixture.sh'))
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

/** A chip in a composer's row that opens a menu, such as the agent to start or the permissions (ADR 0114). */
export function chip(scope: Page | Locator, label: string): Locator {
  return scope.getByRole('button', { name: label, exact: true })
}

/** Picks one of a chip's choices: by its id, such as a preset's, or by its name. */
export async function pick(scope: Page | Locator, label: string, choice: string | { label: string }) {
  const page = 'page' in scope && typeof scope.page === 'function' ? (scope as Locator).page() : (scope as Page)
  await chip(scope, label).click()
  const menu = page.getByRole('menu', { name: label, exact: true })
  await (
    typeof choice === 'string' ? menu.locator(`[data-id="${choice}"]`) : menu.getByRole('menuitemradio', { name: choice.label })
  ).click()
}

/** Picks the agent's model, from the chip that shows it with its effort. */
export async function pickModel(scope: Page | Locator, name: string) {
  await scope.getByRole('button', { name: /^model and effort/ }).click()
  const page = 'page' in scope && typeof scope.page === 'function' ? (scope as Locator).page() : (scope as Page)
  await page.getByRole('menu', { name: 'model' }).getByRole('menuitemradio', { name }).click()
}
