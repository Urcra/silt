import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

// Set by `pnpm dev:daemon`, which asks the running daemon for its address.
const daemon = process.env.SILT_DAEMON_URL

export default defineConfig({
  // Relative, so the page's `<base href>` decides where the assets come from.
  base: './',
  plugins: [react(), tailwindcss()],
  server: daemon
    ? {
        // `changeOrigin` sends the daemon its own address as the host, which
        // its check against DNS rebinding expects.
        proxy: { '/api': { target: daemon, changeOrigin: true, ws: true } },
      }
    : undefined,
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
  },
})
