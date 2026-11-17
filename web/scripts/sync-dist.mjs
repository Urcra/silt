// Moves a fresh build into dist, writing only the files whose bytes changed
// and removing those the build no longer has. Cargo rebuilds silt-web, and
// relinks every binary that embeds it, whenever a file's mtime changes, so
// rewriting an identical UI would cost each `cargo build` its links.
// Usage: node scripts/sync-dist.mjs <fresh> <dist>
import { mkdirSync, readFileSync, readdirSync, rmSync, rmdirSync, writeFileSync } from 'node:fs'
import { join, relative } from 'node:path'

const [fresh, dist] = process.argv.slice(2)
if (!fresh || !dist) {
  console.error('usage: sync-dist.mjs <fresh> <dist>')
  process.exit(2)
}

function files(dir) {
  return readdirSync(dir, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => relative(dir, join(entry.parentPath, entry.name)))
}

function read(path) {
  try {
    return readFileSync(path)
  } catch {
    return null
  }
}

const built = new Set(files(fresh))
mkdirSync(dist, { recursive: true })
for (const stale of files(dist).filter((file) => !built.has(file))) {
  rmSync(join(dist, stale))
}
// index.html last: silt-web's build script watches it, so it changes only
// once the assets it names are in place.
const order = [...built].sort((a, b) => (a === 'index.html') - (b === 'index.html'))
let written = 0
for (const file of order) {
  const bytes = readFileSync(join(fresh, file))
  if (!read(join(dist, file))?.equals(bytes)) {
    mkdirSync(join(dist, file, '..'), { recursive: true })
    writeFileSync(join(dist, file), bytes)
    written += 1
  }
}
// Deepest first, so a directory emptied above goes with its parent.
const dirs = readdirSync(dist, { recursive: true, withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => join(entry.parentPath, entry.name))
  .sort((a, b) => b.length - a.length)
for (const dir of dirs) {
  try {
    rmdirSync(dir)
  } catch {
    // Not empty.
  }
}
rmSync(fresh, { recursive: true, force: true })
console.log(`sync-dist: ${written} of ${built.size} files changed`)
