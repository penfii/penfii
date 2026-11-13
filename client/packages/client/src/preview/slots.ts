import type { ReactNode } from 'react'

/**
 * The one place a demo build plugs its own screens into the app. An ordinary build leaves it empty, so the pages and the
 * shell hold no sample content and no branch on it: the sample Projects reach them as ordinary data (see `sample.ts`),
 * and what has no ordinary form (the overview's invented figures) comes through this slot.
 */
export const previewSlots: {
  /** The overview of a sample Project, or `null` for any other Project. */
  overview?: (projectId: string) => ReactNode | null
} = {}
