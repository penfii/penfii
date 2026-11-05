/**
 * Sample content (Projects, tasks, documents, inbox items) exists for demos only.
 * A real workspace never shows it: turn it on with VITE_PREVIEW_CONTENT=1 in a demo build.
 */
export const previewContent = (import.meta as { env?: Record<string, string | undefined> }).env?.VITE_PREVIEW_CONTENT === '1'

/** The sample list when previews are on, otherwise nothing. */
export function onlyInPreview<T>(items: T[]): T[] {
  return previewContent ? items : []
}
