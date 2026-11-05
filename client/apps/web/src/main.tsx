// Desktop packaging (Tauri) is later; this slice runs as the web client.
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { App } from '@penfii/client'

const root = document.getElementById('root')
if (!root) throw new Error('root element missing')

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
