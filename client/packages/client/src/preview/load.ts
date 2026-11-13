/// <reference types="vite/client" />

/**
 * Installs the sample content of a demo build (`VITE_PREVIEW_CONTENT=1`) before the app renders. The flag is read here,
 * in the condition itself, so that the bundler replaces it with a constant: in an ordinary build the branch is removed
 * together with the dynamic import, and the sample data is not part of the bundle at all.
 */
export async function loadPreview(): Promise<void> {
  if (import.meta.env.VITE_PREVIEW_CONTENT === '1') {
    const { installSample } = await import('./sample')
    installSample()
  }
}
