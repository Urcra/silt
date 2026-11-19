import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

// Set by `pnpm dev:daemon`, which asks the running daemon for its address.
const daemon = process.env.SILT_DAEMON_URL

// The app, and the docs, of which a test reads docs/ui-mocks.md.
const FILES = ['.', '../docs']

export default defineConfig({
  // Relative, so the page's `<base href>` decides where the assets come from.
  base: './',
  plugins: [react(), tailwindcss()],
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
