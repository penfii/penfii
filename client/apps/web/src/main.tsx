// Desktop packaging (Tauri) is later; this slice runs as the web client.
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { App, loadInitialCatalog, loadPreview } from '@penfii/client'

const root = document.getElementById('root')
if (!root) throw new Error('root element missing')

// A demo build installs its sample content first (an ordinary build has nothing to load), and the texts of
// the language the app opens in are fetched, so the first render already has them.
void Promise.all([loadPreview(), loadInitialCatalog()]).then(() => {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  )
})
