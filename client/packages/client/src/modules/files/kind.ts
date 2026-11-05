/**
 * How an attachment is shown. The lists are the browser's side of the server's inline list
 * (`INLINE_MIME` in core-api): a type outside them is a compact file card with a download, whatever
 * its name says, and HTML or SVG are never on them. The `mime` is the one the server detected from
 * the file's bytes.
 */
export type PreviewKind = 'image' | 'video' | 'audio' | 'csv' | 'file'

/** The types the server will show in place. Document blocks and message previews both use these. */
export const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'] as const
export const VIDEO_TYPES = ['video/mp4', 'video/webm'] as const
export const AUDIO_TYPES = ['audio/mpeg', 'audio/ogg', 'audio/wav', 'audio/mp4'] as const

const IMAGE = new Set<string>(IMAGE_TYPES)
const VIDEO = new Set<string>(VIDEO_TYPES)
const AUDIO = new Set<string>(AUDIO_TYPES)

export function previewKind(mime: string | null | undefined): PreviewKind {
  if (!mime) return 'file'
  if (IMAGE.has(mime)) return 'image'
  if (VIDEO.has(mime)) return 'video'
  if (AUDIO.has(mime)) return 'audio'
  if (mime === 'text/csv') return 'csv'
  return 'file'
}

/**
 * The BlockNote block a Document drop or paste should create. Only the types the server will show
 * in place become a media block; SVG, HTML and anything else, including CSV, are a file block.
 */
export function documentBlockType(file: File): 'image' | 'video' | 'audio' | 'file' {
  const kind = previewKind(file.type)
  return kind === 'image' || kind === 'video' || kind === 'audio' ? kind : 'file'
}
