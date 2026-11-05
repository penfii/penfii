import { useEffect, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { isApiError } from '@penfii/api-client'
import { api } from '../../api'
import { queries } from '../../queries'
import { putFile, UploadHttpError } from './upload'

/** How many files one form sends at once. Chat, comments and a Document page all use this. */
export const CONCURRENT_UPLOADS = 3

/** At most `limit` transfers run; the rest wait, and a freed slot goes straight to the next waiter. */
export function uploadQueue(limit: number) {
  let running = 0
  const waiting: Array<() => void> = []
  return {
    async take() {
      if (running < limit) {
        running += 1
        return
      }
      await new Promise<void>((resolve) => waiting.push(resolve))
    },
    free() {
      const next = waiting.shift()
      if (next) next()
      else running -= 1
    },
  }
}

/** Why an upload failed: a refusal retrying cannot fix, or a transfer that did not finish. */
export type UploadFailure = 'tooLarge' | 'rejected' | 'transfer'

/** A client error other than a timeout or a rate limit means the same request will be refused again. */
export function failureOf(error: unknown): UploadFailure {
  const status = error instanceof UploadHttpError ? error.status : isApiError(error) ? error.status : 0
  if (status === 413) return 'tooLarge'
  if (status >= 400 && status < 500 && status !== 408 && status !== 429) return 'rejected'
  return 'transfer'
}

export type UploadItem = {
  localId: string
  file: File
  fileId: string | null
  phase: 'uploading' | 'available' | 'failed'
  failure: UploadFailure | null
  /** The server's size limit when the file was turned away for its size. */
  limit: number | null
  progress: number
  filename: string
  byteSize: number
  mime: string
  previewUrl: string | null
}

/**
 * Files being uploaded for one form: several at once, with progress, retry, cancel and cleanup.
 * Chat messages, Ticket comments and the Ticket body all use it, so they upload the same way.
 *
 * A finished upload waits in `readyIds` for the form to send it with its own request. With
 * `onAvailable` the form binds each file the moment it is stored instead; the item then leaves the
 * list, and a binding that fails shows as a failed upload that can be retried.
 * Uploads that are never sent are deleted when they are removed or the form goes away.
 */
export function useFileUploads({
  workspaceId,
  busy = false,
  onAvailable,
}: {
  workspaceId: string
  busy?: boolean
  onAvailable?: (fileId: string, item: UploadItem) => Promise<void>
}) {
  const limit = useQuery(queries.serverInfo()).data?.limits.fileMaxBytes ?? null
  const aborts = useRef(new Map<string, () => void>())
  const cancelled = useRef(new Set<string>())
  const active = useRef(new Set<string>())
  const slots = useRef(uploadQueue(CONCURRENT_UPLOADS))
  const [uploads, setUploads] = useState<UploadItem[]>([])
  const uploadsRef = useRef(uploads)
  uploadsRef.current = uploads
  const onAvailableRef = useRef(onAvailable)
  onAvailableRef.current = onAvailable

  useEffect(() => () => {
    for (const item of uploadsRef.current) {
      cancelled.current.add(item.localId)
      aborts.current.get(item.localId)?.()
      if (item.previewUrl) URL.revokeObjectURL(item.previewUrl)
      if (item.fileId) void api.deleteFile(item.fileId).catch(() => undefined)
    }
  }, [])

  function remember(localId: string, patch: Partial<UploadItem>) {
    setUploads((current) => current.map((item) => (item.localId === localId ? { ...item, ...patch } : item)))
  }

  function drop(item: UploadItem) {
    if (item.previewUrl) URL.revokeObjectURL(item.previewUrl)
    setUploads((current) => current.filter((entry) => entry.localId !== item.localId))
  }

  async function start(item: UploadItem, retry = false) {
    if (active.current.has(item.localId)) return
    active.current.add(item.localId)
    cancelled.current.delete(item.localId)
    remember(item.localId, { phase: 'uploading', progress: 0, failure: null })
    await slots.current.take()
    try {
      if (cancelled.current.has(item.localId)) return
      if (retry && item.fileId) await api.deleteFile(item.fileId).catch(() => undefined)
      const created = item.fileId && !retry
        ? { id: item.fileId }
        : await api.createFile(workspaceId, { filename: item.filename || 'file', byteSize: item.byteSize })
      if (cancelled.current.has(item.localId)) {
        void api.deleteFile(created.id).catch(() => undefined)
        return
      }
      remember(item.localId, { fileId: created.id })
      const transfer = putFile(created.id, item.file, (ratio) => remember(item.localId, { progress: ratio }))
      aborts.current.set(item.localId, transfer.abort)
      const stored = await transfer.done
      aborts.current.delete(item.localId)
      if (cancelled.current.has(item.localId)) {
        void api.deleteFile(created.id).catch(() => undefined)
        return
      }
      if (stored.state !== 'AVAILABLE') {
        remember(item.localId, { phase: 'failed', progress: 1, failure: 'rejected' })
        return
      }
      const bind = onAvailableRef.current
      if (bind) {
        await bind(created.id, item)
        if (!cancelled.current.has(item.localId)) drop(item)
        return
      }
      remember(item.localId, { phase: 'available', progress: 1, mime: stored.mime ?? item.mime })
    } catch (error) {
      aborts.current.delete(item.localId)
      if (cancelled.current.has(item.localId)) return
      if (error instanceof Error && error.message === 'aborted') return
      remember(item.localId, { phase: 'failed', failure: failureOf(error), limit })
    } finally {
      active.current.delete(item.localId)
      slots.current.free()
    }
  }

  function addFiles(files: File[]) {
    if (busy || files.length === 0) return
    const next = files.map((file): UploadItem => ({
      localId: crypto.randomUUID(),
      file,
      fileId: null,
      phase: limit !== null && file.size > limit ? 'failed' : 'uploading',
      failure: limit !== null && file.size > limit ? 'tooLarge' : null,
      limit,
      progress: 0,
      filename: file.name || 'image',
      byteSize: file.size,
      mime: file.type,
      previewUrl: file.type.startsWith('image/') ? URL.createObjectURL(file) : null,
    }))
    setUploads((current) => [...current, ...next])
    for (const item of next) if (item.phase === 'uploading') void start(item)
  }

  function remove(item: UploadItem) {
    if (busy) return
    cancelled.current.add(item.localId)
    aborts.current.get(item.localId)?.()
    aborts.current.delete(item.localId)
    if (item.fileId) void api.deleteFile(item.fileId).catch(() => undefined)
    drop(item)
  }

  /** After the form was sent: its files now belong to the message or comment; anything unfinished is deleted. */
  function settle() {
    for (const item of uploadsRef.current) {
      if (item.previewUrl) URL.revokeObjectURL(item.previewUrl)
      if (item.phase !== 'available' && item.fileId) void api.deleteFile(item.fileId).catch(() => undefined)
    }
    setUploads([])
  }

  return {
    uploads,
    readyIds: uploads.flatMap((item) => (item.phase === 'available' && item.fileId ? [item.fileId] : [])),
    uploading: uploads.some((item) => item.phase === 'uploading'),
    /** A file failed: the form cannot be sent until it is retried or removed. The text is not touched. */
    failed: uploads.some((item) => item.phase === 'failed'),
    /** Something is not ready to send (still uploading, or failed). */
    blocking: uploads.some((item) => item.phase !== 'available'),
    addFiles,
    retry: (item: UploadItem) => void start(item, true),
    remove,
    settle,
  }
}
