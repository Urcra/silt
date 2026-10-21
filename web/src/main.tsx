import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import { takeTokenFromUrl } from './api'
import { legacyPath } from './legacy'
import { applyTheme, storedTheme } from './theme'

applyTheme(storedTheme())
const moved = legacyPath(window.location.hash)
if (moved) window.history.replaceState(null, '', moved)

void takeTokenFromUrl().finally(() => {
  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  )
})
