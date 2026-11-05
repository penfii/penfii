import { createExtension, type BlockNoteEditor } from '@blocknote/core'
import { Plugin, PluginKey } from '@tiptap/pm/state'
import { Decoration, DecorationSet } from '@tiptap/pm/view'
import { api } from '../../api'
import { documentBlockType } from '../files/kind'
import { openMediaViewer } from '../files/mediaViewer'
import { putFile } from '../files/upload'
import { CONCURRENT_UPLOADS, failureOf, uploadQueue, type UploadFailure } from '../files/useFileUploads'

/** What an upload in a Document body is doing. It lives on the block that will hold the file. */
export type DocUpload = {
  blockId: string
  name: string
  phase: 'uploading' | 'failed'
  progress: number
  failure: UploadFailure | null
  limit: number | null
}

type Entry = DocUpload & {
  file: File
  fileId: string | null
  abort: (() => void) | null
  cancelled: boolean
}

export type DocUploadLabels = {
  uploading: string
  cancel: string
  retry: string
  remove: string
  tooLarge: (limit: number) => string
  rejected: string
  failed: string
}

/**
 * The uploads of one Document editor. BlockNote puts an empty file, image, video or audio block
 * where a file is dropped, pasted or picked, and waits for `uploadFile` to say where the file is.
 * This keeps that promise open while the file is sent and while it can be retried, so the block
 * itself can show progress, cancel, retry and remove (see `docUploadsExtension`) and the text of
 * the Document is never touched by a failure.
 */
export class DocUploads {
  private readonly entries = new Map<string, Entry>()
  private readonly listeners = new Set<() => void>()
  private counter = 0
  private readonly slots = uploadQueue(CONCURRENT_UPLOADS)

  constructor(
    private readonly options: {
      workspaceId: string
      /** Binds the stored file to the page so everyone in the room can fetch it. */
      attach: (fileId: string) => Promise<void>
      limit: () => number | null
      /** The block holding a cancelled or removed upload goes away with it. */
      removeBlock: (blockId: string) => void
      /** Writes the stored file's address onto its block once the upload has finished. */
      setUrl: (blockId: string, url: string) => void
    },
  ) {}

  /**
   * Passed to BlockNote as `uploadFile`. BlockNote inserts the next file only after this settles,
   * so it settles at once with an empty address and the block keeps showing this upload. The real
   * address is written by `setUrl` when the file is stored; a failure never writes one, and does
   * not hold the files dropped with it.
   */
  uploadFile = (file: File, blockId?: string): Promise<string> => {
    const id = blockId ?? `upload-${String((this.counter += 1))}`
    const entry: Entry = { blockId: id, name: file.name, phase: 'uploading', progress: 0, failure: null, limit: null, file, fileId: null, abort: null, cancelled: false }
    this.entries.set(id, entry)
    void this.run(entry)
    return Promise.resolve('')
  }

  list(): DocUpload[] {
    return [...this.entries.values()]
  }

  get(blockId: string): DocUpload | undefined {
    return this.entries.get(blockId)
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  retry(blockId: string) {
    const entry = this.entries.get(blockId)
    if (entry?.phase === 'failed' && entry.failure === 'transfer') void this.run(entry)
  }

  /** Stops an upload that is running or failed, deletes what was stored and removes the block. */
  cancel(blockId: string) {
    const entry = this.entries.get(blockId)
    if (!entry) return
    entry.cancelled = true
    entry.abort?.()
    if (entry.fileId) void api.deleteFile(entry.fileId).catch(() => undefined)
    this.entries.delete(blockId)
    this.options.removeBlock(blockId)
    this.emit()
  }

  /** The editor goes away: what is still on its way is dropped. */
  dispose() {
    for (const entry of [...this.entries.values()]) {
      entry.cancelled = true
      entry.abort?.()
      if (entry.fileId) void api.deleteFile(entry.fileId).catch(() => undefined)
    }
    this.entries.clear()
    this.emit()
  }

  private emit() {
    for (const listener of this.listeners) listener()
  }

  private async run(entry: Entry) {
    entry.cancelled = false
    entry.phase = 'uploading'
    entry.progress = 0
    entry.failure = null
    entry.limit = this.options.limit()
    this.emit()
    if (entry.limit !== null && entry.file.size > entry.limit) {
      entry.phase = 'failed'
      entry.failure = 'tooLarge'
      this.emit()
      return
    }
    await this.slots.take()
    try {
      if (entry.cancelled) return
      if (entry.fileId) await api.deleteFile(entry.fileId).catch(() => undefined)
      const created = await api.createFile(this.options.workspaceId, { filename: entry.file.name || 'file', byteSize: entry.file.size })
      if (entry.cancelled) {
        void api.deleteFile(created.id).catch(() => undefined)
        return
      }
      entry.fileId = created.id
      let last = 0
      const transfer = putFile(created.id, entry.file, (ratio) => {
        const whole = Math.round(ratio * 100)
        if (whole === last) return
        last = whole
        entry.progress = ratio
        this.emit()
      })
      entry.abort = transfer.abort
      const stored = await transfer.done
      entry.abort = null
      if (entry.cancelled) {
        void api.deleteFile(created.id).catch(() => undefined)
        return
      }
      if (stored.state !== 'AVAILABLE') {
        entry.phase = 'failed'
        entry.failure = 'rejected'
        this.emit()
        return
      }
      await this.options.attach(created.id)
      if (entry.cancelled) return
      this.entries.delete(entry.blockId)
      this.options.setUrl(entry.blockId, `/v1/files/${created.id}/content`)
      this.emit()
    } catch (error) {
      entry.abort = null
      if (entry.cancelled || (error instanceof Error && error.message === 'aborted')) return
      entry.phase = 'failed'
      entry.failure = failureOf(error)
      this.emit()
    } finally {
      this.slots.free()
    }
  }
}

const key = new PluginKey('docUploads')

function failureText(upload: DocUpload, labels: DocUploadLabels): string {
  if (upload.failure === 'tooLarge' && upload.limit !== null) return labels.tooLarge(upload.limit)
  if (upload.failure === 'rejected' || upload.failure === 'tooLarge') return labels.rejected
  return labels.failed
}

/**
 * Shows each running or failed upload on its own block: a bar with the file name, the percentage and
 * cancel while it runs; the reason with retry and remove once it failed. Also opens the shared
 * viewer when an image or a video of the page is double-clicked.
 */
export function docUploadsExtension(uploads: DocUploads, labels: () => DocUploadLabels) {
  // The bar element of each upload, kept so progress repaints it in place instead of rebuilding it.
  const live = new Map<string, HTMLElement>()
  const paint = (element: HTMLElement, upload: DocUpload) => {
    const text = labels()
    element.dataset.state = upload.phase
    const percent = Math.round(upload.progress * 100)
    element.querySelector<HTMLElement>('.doc-upload-name')!.textContent = upload.name
    element.querySelector<HTMLElement>('.doc-upload-status')!.textContent = upload.phase === 'uploading' ? `${text.uploading} · ${String(percent)}%` : failureText(upload, text)
    element.querySelector<HTMLElement>('.doc-upload-bar')!.style.width = `${String(percent)}%`
    element.querySelector<HTMLElement>('.doc-upload-track')!.hidden = upload.phase !== 'uploading'
    element.setAttribute('role', upload.phase === 'failed' ? 'alert' : 'status')
  }
  return createExtension({
    key: 'docUploads',
    prosemirrorPlugins: [
      new Plugin({
        key,
        view(view) {
          let signature = ''
          const unsubscribe = uploads.subscribe(() => {
            const next = uploads.list().map((upload) => `${upload.blockId}:${upload.phase}:${String(upload.failure)}`).join('|')
            if (next !== signature) {
              signature = next
              view.dispatch(view.state.tr.setMeta(key, 'refresh'))
            }
            for (const upload of uploads.list()) {
              const element = live.get(upload.blockId)
              if (element) paint(element, upload)
            }
            for (const [blockId, element] of live) if (!uploads.get(blockId) && !element.isConnected) live.delete(blockId)
          })
          return { destroy: unsubscribe }
        },
        props: {
          decorations(state) {
            const list = uploads.list()
            if (list.length === 0) return null
            const decorations: Decoration[] = []
            state.doc.descendants((node, pos) => {
              if (node.type.name !== 'blockContainer') return true
              const upload = uploads.get(String(node.attrs.id))
              if (!upload) return true
              decorations.push(Decoration.node(pos, pos + node.nodeSize, { 'data-upload': upload.phase }))
              const after = pos + 1 + (node.firstChild?.nodeSize ?? 0)
              decorations.push(
                Decoration.widget(
                  after,
                  () => {
                    const text = labels()
                    const element = document.createElement('div')
                    element.className = 'doc-upload'
                    element.contentEditable = 'false'
                    element.innerHTML = '<span class="doc-upload-name"></span><span class="doc-upload-status"></span><span class="doc-upload-track"><span class="doc-upload-bar"></span></span><span class="doc-upload-actions"></span>'
                    const actions = element.querySelector<HTMLElement>('.doc-upload-actions')!
                    const button = (label: string, act: () => void) => {
                      const b = document.createElement('button')
                      b.type = 'button'
                      b.textContent = label
                      b.setAttribute('aria-label', `${label}: ${upload.name}`)
                      // Acting on mousedown keeps the click from being lost when the bar is repainted.
                      b.addEventListener('mousedown', (event) => {
                        event.preventDefault()
                        event.stopPropagation()
                        act()
                      })
                      actions.append(b)
                    }
                    if (upload.phase === 'uploading') button(text.cancel, () => uploads.cancel(upload.blockId))
                    else {
                      if (upload.failure === 'transfer') button(text.retry, () => uploads.retry(upload.blockId))
                      button(text.remove, () => uploads.cancel(upload.blockId))
                    }
                    paint(element, upload)
                    live.set(upload.blockId, element)
                    return element
                  },
                  { key: `${upload.blockId}:${upload.phase}:${String(upload.failure)}`, side: 1, ignoreSelection: true, stopEvent: () => true },
                ),
              )
              return true
            })
            return DecorationSet.create(state.doc, decorations)
          },
          handleDOMEvents: {
            dblclick(_view, event) {
              const target = event.target
              if (!(target instanceof HTMLImageElement || target instanceof HTMLVideoElement)) return false
              const src = target.getAttribute('src') ?? ''
              if (!src.startsWith('/v1/files/')) return false
              const name = target.getAttribute('alt') || target.getAttribute('aria-label') || ''
              openMediaViewer([{ kind: target instanceof HTMLVideoElement ? 'video' : 'image', src, name }])
              return true
            },
          },
        },
      }),
    ],
  })
}

/** Inserts blocks for files that did not land on the text (a drop on the page margin), then uploads them. */
export function insertFiles(editor: BlockNoteEditor<any, any, any>, uploads: DocUploads, files: File[]) {
  const document = editor.document
  let reference = editor.getTextCursorPosition().block ?? document[document.length - 1]
  if (!reference) return
  for (const file of files) {
    const inserted = editor.insertBlocks([{ type: documentBlockType(file), props: { name: file.name } } as never], reference, 'after')[0]
    if (!inserted) continue
    reference = inserted
    void uploads.uploadFile(file, inserted.id)
  }
}
