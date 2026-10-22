import { defineConfig } from '@playwright/test'

// Playwright's own Chromium on every machine, so the screen references match
// wherever they're taken; a cloud session's setup script installs it
// (scripts/cloud-setup.sh). SILT_E2E_CHROMIUM picks another.
const executablePath = process.env.SILT_E2E_CHROMIUM

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
