import { defineConfig } from '@playwright/test'

// Cloud sessions ship one Chromium, older than the one this Playwright
// downloads, at a fixed path.
const executablePath =
  process.env.SILT_E2E_CHROMIUM ?? (process.env.CLAUDE_CODE_REMOTE === 'true' ? '/opt/pw-browsers/chromium' : undefined)

export default defineConfig({
  testDir: './e2e',
  globalSetup: './e2e/global-setup.ts',
  fullyParallel: false,
  workers: 1,
  reporter: 'list',
  use: { browserName: 'chromium', trace: 'retain-on-failure', launchOptions: { executablePath } },
  // One reference per screen and variant, whatever machine takes it (ADR 0039).
  snapshotPathTemplate: '{testDir}/{testFileName}-snapshots/{arg}{ext}',
})
