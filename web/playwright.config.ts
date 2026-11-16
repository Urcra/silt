import { defineConfig } from '@playwright/test'

// Playwright's own Chromium on every machine, so the screen references match
// wherever they're taken; a cloud session's setup script installs it
// (scripts/cloud-setup.sh). SILT_E2E_CHROMIUM picks another.
const executablePath = process.env.SILT_E2E_CHROMIUM

export default defineConfig({
  testDir: './e2e',
  globalSetup: './e2e/global-setup.ts',
  // Each spec file sets up its own daemons and servers, in a temporary
  // SILT_HOME on free ports, so two files run at once; a file's tests share
  // its setup, so they run in order. SILT_E2E_WORKERS sets how many.
  fullyParallel: false,
  workers: Number(process.env.SILT_E2E_WORKERS ?? 2),
  projects: [
    { name: 'e2e', testIgnore: /perf\.spec\.ts/ },
    // Timed (ADR 0034), so alone, once the others are done.
    { name: 'perf', testMatch: /perf\.spec\.ts/, dependencies: ['e2e'] },
  ],
  reporter: 'list',
  use: { browserName: 'chromium', trace: 'retain-on-failure', launchOptions: { executablePath } },
  // One reference per screen and variant, whatever machine takes it (ADR 0039).
  snapshotPathTemplate: '{testDir}/{testFileName}-snapshots/{arg}{ext}',
})
