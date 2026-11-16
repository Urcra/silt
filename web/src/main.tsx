import { StrictMode, Suspense, lazy } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import { linkToken } from './links'
import { takeTokenFromUrl } from './api'
import { legacyPath } from './legacy'
import { hosted } from './served'
import { applyTheme, storedTheme } from './theme'

applyTheme(storedTheme())
const moved = legacyPath(window.location.hash)
if (moved) window.history.replaceState(null, '', moved)

// A read-only link to one artifact is this bundle too, with nothing of the app but the artifact (ADR 0073).
const linked = document.querySelector('meta[name="silt-link"]') ? linkToken(window.location.pathname) : null
// Loaded only for a link, so the app's first screen doesn't carry the artifact viewer.
const LinkedArtifact = lazy(() => import('./components/LinkedArtifact').then((module) => ({ default: module.LinkedArtifact })))
if (linked) {
  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <Suspense fallback={null}>
        <LinkedArtifact token={linked} />
      </Suspense>
    </StrictMode>,
  )
} else {
  void takeTokenFromUrl().finally(() => {
    createRoot(document.getElementById('root')!).render(
      <StrictMode>
        <App />
      </StrictMode>,
    )
  })
  // Installed to a phone's home screen, the hosted app runs with its worker (ADR 0028).
  if (hosted() && 'serviceWorker' in navigator) void navigator.serviceWorker.register('sw.js').catch(() => undefined)
}
