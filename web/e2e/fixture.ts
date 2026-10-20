import { execFileSync } from 'node:child_process'
import { readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

export interface Fixture {
  url: string
  siltHome: string
  work: string
  thread: string
  fork: string
}

const FILE = join(import.meta.dirname, '.fixture.json')
const SILT = join(import.meta.dirname, '..', '..', 'target', 'debug', 'silt')

export function createFixture(): void {
  const output = execFileSync(join(import.meta.dirname, 'fixture.sh'), { encoding: 'utf8' })
  writeFileSync(FILE, output.trim().split('\n').pop() ?? '')
}

export function fixture(): Fixture {
  return JSON.parse(readFileSync(FILE, 'utf8')) as Fixture
}

export function removeFixture(): void {
  const { siltHome, work } = fixture()
  execFileSync(SILT, ['daemon', 'stop'], { env: { ...process.env, SILT_HOME: siltHome } })
  rmSync(work, { recursive: true, force: true })
  rmSync(FILE, { force: true })
}
