import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import license from 'rollup-plugin-license'
import { defineConfig } from 'vitest/config'

// Set by `pnpm dev:daemon`, which asks the running daemon for its address.
const daemon = process.env.SILT_DAEMON_URL

// Licences that may be bundled into Silt's AGPL-3.0 UI. A dependency under
// anything else fails the build until it's checked and added here.
const ALLOWED_LICENCES = new Set([
  'MIT',
  'MIT-0',
  'ISC',
  '0BSD',
  'BSD-2-Clause',
  'BSD-3-Clause',
  'Apache-2.0',
  'Zlib',
  'Unlicense',
  'CC0-1.0',
  'OFL-1.1',
  'MPL-2.0',
  'GPL-3.0-or-later',
  'AGPL-3.0-only',
])

// Packages whose package.json names no licence, by the licence file they ship.
const UNLABELLED: Record<string, string> = { khroma: 'MIT' }

// Packages bundled through CSS, which the licence plugin doesn't see.
const CSS_PACKAGES = ['@fontsource/ibm-plex-sans', '@fontsource/ibm-plex-mono']

type Notice = { name: string; version: string; license: string; text: string }

// A package's licence file, as installed.
function licenceText(name: string): string {
  const dir = join('node_modules', name)
  const file = readdirSync(dir).find((entry) => /^licen[cs]e(\.|$)/i.test(entry))
  return file ? readFileSync(join(dir, file), 'utf8') : ''
}

function noticeOf(name: string): Notice {
  const manifest = JSON.parse(readFileSync(join('node_modules', name, 'package.json'), 'utf8'))
  return { name, version: manifest.version, license: manifest.license, text: licenceText(name) }
}

function noticesText(notices: Notice[]): string {
  const missing = notices.filter((notice) => !notice.text.trim()).map((notice) => notice.name)
  if (missing.length > 0) throw new Error(`no licence text found for ${missing.join(', ')}`)
  return notices
    .sort((a, b) => a.name.localeCompare(b.name))
    .map(({ name, version, license, text }) => `${name} ${version}\nLicense: ${license}\n\n${text.trim()}`)
    .join('\n\n---\n\n')
}

// Whether an SPDX expression of ORs of ANDs leaves a choice made only of
// allowed licences. Anything it can't read is refused, for a person to check.
function allowedLicence(expression: string | null | undefined): boolean {
  if (!expression || /[()]/.test(expression.replace(/^\((.*)\)$/, '$1'))) return false
  return expression
    .replace(/^\((.*)\)$/, '$1')
    .split(' OR ')
    .some((choice) => choice.split(' AND ').every((id) => ALLOWED_LICENCES.has(id.trim())))
}

// What the licence plugin found, for the next plugin to put in the build.
let notices: string | undefined

// The app, and the docs, of which a test reads docs/ui-mocks.md.
const FILES = ['.', '../docs']

export default defineConfig({
  // Relative, so the page's `<base href>` decides where the assets come from.
  base: './',
  plugins: [
    react(),
    tailwindcss(),
    // Every package bundled into the UI, with its licence text, served beside it.
    license({
      thirdParty: {
        allow: {
          test: (dependency) => allowedLicence(dependency.license ?? UNLABELLED[dependency.name ?? '']),
          failOnUnlicensed: true,
          failOnViolation: true,
        },
        output: (dependencies) => {
          const bundled = dependencies.map((dependency) => {
            const name = dependency.name ?? ''
            return {
              name,
              version: dependency.version ?? '',
              license: dependency.license ?? UNLABELLED[name] ?? '',
              text: dependency.licenseText || licenceText(name),
            }
          })
          notices = noticesText([...bundled, ...CSS_PACKAGES.map(noticeOf)])
        },
      },
    }),
    {
      name: 'third-party-notices',
      generateBundle() {
        if (notices !== undefined) {
          this.emitFile({ type: 'asset', fileName: 'THIRD-PARTY-NOTICES.txt', source: notices })
        }
      },
    },
  ],
  build: {
    // Mermaid's layout engine, elk, is one 1.5 MB module. It loads only when a diagram is drawn, as Mermaid itself does.
    chunkSizeWarningLimit: 1600,
  },
  server: daemon
    ? {
        // `changeOrigin` sends the daemon its own address as the host, which
        // its check against DNS rebinding expects.
        proxy: { '/api': { target: daemon, changeOrigin: true, ws: true } },
        fs: { allow: FILES },
      }
    : { fs: { allow: FILES } },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
  },
})
