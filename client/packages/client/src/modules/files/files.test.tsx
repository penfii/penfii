import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../../i18n/I18nProvider'
import { QueryWrapper } from '../../test/queryWrapper'

const api = vi.hoisted(() => ({ createFile: vi.fn(), deleteFile: vi.fn(), serverInfo: vi.fn() }))
const transfers = vi.hoisted(() => ({ putFile: vi.fn() }))
vi.mock('../../api', () => ({ api }))
vi.mock('./upload', async (importOriginal) => ({ ...(await importOriginal<typeof import('./upload')>()), ...transfers }))

import { FileDropZone } from './FileDropZone'
import { AttachmentList, UploadCards } from './AttachmentList'
import { useFileUploads } from './useFileUploads'
import { MediaViewerHost, openMediaViewer } from './mediaViewer'

const file = (name: string, type = 'text/plain') => new File(['x'], name, { type })

let nextId = 0
beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  nextId = 0
  URL.createObjectURL = vi.fn(() => 'blob:preview')
  URL.revokeObjectURL = vi.fn()
  api.createFile.mockImplementation(async () => ({ id: `file-${String(++nextId)}` }))
  api.deleteFile.mockResolvedValue(undefined)
  api.serverInfo.mockResolvedValue({ apiVersion: '', features: [], limits: { messageBodyMaxBytes: null, fileMaxBytes: 10, sessionTtlSeconds: null } })
  transfers.putFile.mockImplementation((_id: string, _file: File, onProgress: (ratio: number) => void) => {
    onProgress(0.5)
    return { done: Promise.resolve({ state: 'AVAILABLE', mime: 'text/plain' }), abort: vi.fn() }
  })
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('useFileUploads', () => {
  it('turns a file over the server limit away before sending anything', async () => {
    const { result } = renderHook(() => useFileUploads({ workspaceId: 'w1' }), { wrapper: QueryWrapper })
    await waitFor(() => expect(api.serverInfo).toHaveBeenCalled())
    await act(() => new Promise((resolve) => setTimeout(resolve, 0)))
    act(() => result.current.addFiles([new File(['x'.repeat(11)], 'big.bin')]))
    expect(result.current.uploads[0]).toMatchObject({ phase: 'failed', failure: 'tooLarge', limit: 10 })
    expect(api.createFile).not.toHaveBeenCalled()
  })

  it('uploads several files and hands the finished ids to the form', async () => {
    const { result } = renderHook(() => useFileUploads({ workspaceId: 'w1' }), { wrapper: QueryWrapper })
    act(() => result.current.addFiles([file('a.txt'), file('b.pdf')]))
    await waitFor(() => expect(result.current.readyIds).toEqual(['file-1', 'file-2']))
    expect(result.current.uploading).toBe(false)
    expect(api.createFile).toHaveBeenCalledWith('w1', { filename: 'a.txt', byteSize: 1 })
    // Sent: nothing is deleted for the finished ones, the list empties.
    act(() => result.current.settle())
    expect(result.current.uploads).toEqual([])
    expect(api.deleteFile).not.toHaveBeenCalled()
  })

  it('deletes a file removed before it was sent and ignores adds while busy', async () => {
    const { result, rerender } = renderHook(({ busy }) => useFileUploads({ workspaceId: 'w1', busy }), { initialProps: { busy: false }, wrapper: QueryWrapper })
    act(() => result.current.addFiles([file('a.txt')]))
    await waitFor(() => expect(result.current.readyIds).toEqual(['file-1']))
    act(() => result.current.remove(result.current.uploads[0]!))
    expect(api.deleteFile).toHaveBeenCalledWith('file-1')
    expect(result.current.uploads).toEqual([])
    rerender({ busy: true })
    act(() => result.current.addFiles([file('b.txt')]))
    expect(result.current.uploads).toEqual([])
  })

  it('marks a rejected or failed upload and retries it with a fresh file', async () => {
    transfers.putFile.mockImplementationOnce(() => ({ done: Promise.reject(new Error('upload failed')), abort: vi.fn() }))
    const { result } = renderHook(() => useFileUploads({ workspaceId: 'w1' }), { wrapper: QueryWrapper })
    act(() => result.current.addFiles([file('a.txt')]))
    await waitFor(() => expect(result.current.uploads[0]?.phase).toBe('failed'))
    expect(result.current.readyIds).toEqual([])
    act(() => result.current.retry(result.current.uploads[0]!))
    await waitFor(() => expect(result.current.readyIds).toEqual(['file-2']))
    expect(api.deleteFile).toHaveBeenCalledWith('file-1')
  })

  it('binds each file as soon as it is stored and drops it from the list; a failed binding can be retried', async () => {
    const bound: string[] = []
    let fail = true
    const onAvailable = vi.fn(async (fileId: string) => {
      if (fail) throw new Error('nope')
      bound.push(fileId)
    })
    const { result } = renderHook(() => useFileUploads({ workspaceId: 'w1', onAvailable }), { wrapper: QueryWrapper })
    act(() => result.current.addFiles([file('a.txt')]))
    await waitFor(() => expect(result.current.uploads[0]?.phase).toBe('failed'))
    fail = false
    act(() => result.current.retry(result.current.uploads[0]!))
    await waitFor(() => expect(result.current.uploads).toEqual([]))
    expect(bound).toEqual(['file-2'])
  })

  it('deletes unsent uploads when the form goes away', async () => {
    const { result, unmount } = renderHook(() => useFileUploads({ workspaceId: 'w1' }), { wrapper: QueryWrapper })
    act(() => result.current.addFiles([file('a.txt')]))
    await waitFor(() => expect(result.current.readyIds).toEqual(['file-1']))
    unmount()
    expect(api.deleteFile).toHaveBeenCalledWith('file-1')
  })
})

describe('FileDropZone', () => {
  it('takes dropped and pasted files, but not text, and nothing while disabled', () => {
    const onFiles = vi.fn()
    const { rerender } = render(<I18nProvider><FileDropZone onFiles={onFiles}><textarea aria-label="body" /></FileDropZone></I18nProvider>)
    const box = screen.getByLabelText('body')
    const a = file('a.txt')
    fireEvent.drop(box, { dataTransfer: { types: ['Files'], files: [a] } })
    expect(onFiles).toHaveBeenLastCalledWith([a])
    const b = file('b.png', 'image/png')
    fireEvent.paste(box, { clipboardData: { files: [b], types: ['Files'] } })
    expect(onFiles).toHaveBeenLastCalledWith([b])
    onFiles.mockClear()
    fireEvent.drop(box, { dataTransfer: { types: ['text/plain'], files: [] } })
    fireEvent.paste(box, { clipboardData: { files: [], types: ['text/plain'] } })
    expect(onFiles).not.toHaveBeenCalled()
    rerender(<I18nProvider><FileDropZone onFiles={onFiles} disabled><textarea aria-label="body" /></FileDropZone></I18nProvider>)
    fireEvent.drop(screen.getByLabelText('body'), { dataTransfer: { types: ['Files'], files: [a] } })
    expect(onFiles).not.toHaveBeenCalled()
  })

  it('leaves a drop on the document editor to the editor', () => {
    const onFiles = vi.fn()
    render(<I18nProvider><FileDropZone onFiles={onFiles}><div className="bn-editor"><p>page</p></div></FileDropZone></I18nProvider>)
    fireEvent.drop(screen.getByText('page'), { dataTransfer: { types: ['Files'], files: [file('a.png', 'image/png')] } })
    expect(onFiles).not.toHaveBeenCalled()
  })
})

describe('attachment cards', () => {
  it('shows an image preview and a download link for each attachment, and the screen’s own actions', () => {
    render(
      <I18nProvider>
        <AttachmentList
          items={[
            { id: 'a1', fileId: 'f1', filename: 'shot.png', byteSize: 2048, mime: 'image/png', state: 'AVAILABLE' },
            { id: 'a2', fileId: 'f2', filename: 'notes.txt', byteSize: 12, mime: 'application/octet-stream', state: 'AVAILABLE' },
          ]}
          actions={(item) => <button type="button">act {item.filename}</button>}
        />
      </I18nProvider>,
    )
    expect(screen.getByAltText('shot.png').getAttribute('src')).toBe('/v1/attachments/a1/content')
    expect(screen.getAllByRole('link').map((link) => link.getAttribute('href'))).toEqual(['/v1/attachments/a1/content', '/v1/attachments/a2/content'])
    expect(screen.getByText(/2\.0 KB/)).toBeTruthy()
    expect(screen.getByText('act notes.txt')).toBeTruthy()
  })

  it('offers retry on a transfer that did not finish, and remove on every upload', () => {
    const onRetry = vi.fn()
    const onRemove = vi.fn()
    const item = { localId: 'l1', file: file('a.txt'), fileId: null, phase: 'failed' as const, failure: 'transfer' as const, limit: null, progress: 0, filename: 'a.txt', byteSize: 1, mime: '', previewUrl: null }
    render(
      <I18nProvider>
        <UploadCards items={[item]} onRetry={onRetry} onRemove={onRemove} />
      </I18nProvider>,
    )
    fireEvent.click(screen.getByText('Retry'))
    fireEvent.click(screen.getByRole('button', { name: 'Remove file: a.txt' }))
    expect(onRetry).toHaveBeenCalledWith(item)
    expect(onRemove).toHaveBeenCalledWith(item)
  })

  it('names the size limit and offers no retry for a file the server refuses', () => {
    const item = { localId: 'l1', file: file('big.bin'), fileId: null, phase: 'failed' as const, failure: 'tooLarge' as const, limit: 10 * 1024 * 1024, progress: 0, filename: 'big.bin', byteSize: 11 * 1024 * 1024, mime: '', previewUrl: null }
    render(
      <I18nProvider>
        <UploadCards items={[item]} onRetry={vi.fn()} onRemove={vi.fn()} />
      </I18nProvider>,
    )
    expect(screen.getByRole('alert').textContent).toBe('This file is over the 10.0 MB limit. Upload a smaller file.')
    expect(screen.queryByText('Retry')).toBeNull()
  })
})

describe('image viewer', () => {
  it('closes on Escape without the Escape reaching the screen behind it', () => {
    const behind = vi.fn()
    render(
      <I18nProvider>
        <div onKeyDown={behind}>
          <button type="button">behind</button>
        </div>
        <MediaViewerHost />
      </I18nProvider>,
    )
    act(() => { openMediaViewer([{ kind: 'image', src: '/v1/attachments/a1/content', name: 'shot.png' }]) })
    expect(screen.getByRole('dialog')).toBeTruthy()
    fireEvent.keyDown(screen.getByText('behind'), { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(behind).not.toHaveBeenCalled()
  })
})

describe('file sharing presentation', () => {
  it('shows each type the way it should and never inlines active content', async () => {
    const { previewKind } = await import('./kind')
    expect(['image/png', 'image/webp', 'video/mp4', 'video/webm', 'audio/mpeg', 'audio/wav', 'text/csv', 'application/pdf', 'text/html', 'image/svg+xml', null].map(previewKind))
      .toEqual(['image', 'image', 'video', 'video', 'audio', 'audio', 'csv', 'file', 'file', 'file', 'file'])
  })

  it('puts several images in a grid, a video behind a poster with a play button, audio in a player and the rest in cards', () => {
    render(
      <I18nProvider>
        <AttachmentList
          items={[
            { id: 'i1', fileId: 'f1', filename: 'a.png', byteSize: 10, mime: 'image/png', state: 'AVAILABLE' },
            { id: 'i2', fileId: 'f2', filename: 'b.png', byteSize: 10, mime: 'image/png', state: 'AVAILABLE' },
            { id: 'v1', fileId: 'f3', filename: 'clip.mp4', byteSize: 10, mime: 'video/mp4', state: 'AVAILABLE' },
            { id: 'au', fileId: 'f4', filename: 'voice.mp3', byteSize: 10, mime: 'audio/mpeg', state: 'AVAILABLE' },
            { id: 'p1', fileId: 'f5', filename: 'doc.pdf', byteSize: 10, mime: 'application/pdf', state: 'AVAILABLE' },
            { id: 'h1', fileId: 'f6', filename: 'page.html', byteSize: 10, mime: 'application/octet-stream', state: 'AVAILABLE' },
          ]}
        />
      </I18nProvider>,
    )
    expect(document.querySelectorAll('.attachment-grid img')).toHaveLength(2)
    const video = document.querySelector('video') as HTMLVideoElement
    expect(video.autoplay).toBe(false)
    expect(video.controls).toBe(false)
    expect(screen.getByRole('button', { name: /Play: clip\.mp4/ })).toBeTruthy()
    const audio = document.querySelector('audio') as HTMLAudioElement
    expect(audio.autoplay).toBe(false)
    expect(screen.getByText('doc.pdf').closest('li')?.className).toContain('file')
    expect(screen.getByText('page.html').closest('li')?.className).toContain('file')
    expect(document.querySelectorAll('iframe, object, embed')).toHaveLength(0)
  })

  it('falls back to a file card when the browser cannot play the video', () => {
    render(
      <I18nProvider>
        <AttachmentList items={[{ id: 'v1', fileId: 'f3', filename: 'clip.mp4', byteSize: 10, mime: 'video/mp4', state: 'AVAILABLE' }]} />
      </I18nProvider>,
    )
    fireEvent.error(document.querySelector('video') as HTMLVideoElement)
    expect(document.querySelector('video')).toBeNull()
    expect(screen.getByText('clip.mp4').closest('li')?.className).toContain('file')
    expect(screen.getByText(/cannot play this video/)).toBeTruthy()
    expect(screen.getByRole('link', { name: /clip\.mp4/ }).getAttribute('href')).toBe('/v1/attachments/v1/content')
  })

  it('opens images and videos of a list in one viewer and pages through them', () => {
    render(
      <I18nProvider>
        <AttachmentList
          items={[
            { id: 'i1', fileId: 'f1', filename: 'a.png', byteSize: 10, mime: 'image/png', state: 'AVAILABLE' },
            { id: 'v1', fileId: 'f3', filename: 'clip.mp4', byteSize: 10, mime: 'video/mp4', state: 'AVAILABLE' },
          ]}
        />
        <MediaViewerHost />
      </I18nProvider>,
    )
    fireEvent.click(screen.getByRole('button', { name: /Enlarge image: a\.png/ }))
    expect(screen.getByRole('dialog').querySelector('img')?.getAttribute('src')).toBe('/v1/attachments/i1/content')
    expect(screen.getByText('1 / 2')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    expect(screen.getByRole('dialog').querySelector('video')?.getAttribute('src')).toBe('/v1/attachments/v1/content')
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
  })
})

describe('CSV reading', () => {
  it('reads quotes, doubled quotes, CRLF, a BOM and other delimiters, and stops at the limits', async () => {
    const { parseCsv } = await import('./csv')
    expect(parseCsv('﻿name,note\r\n"Kim, J","say ""hi"""\r\nLee,ok', { rows: 10, columns: 10 }).rows)
      .toEqual([['name', 'note'], ['Kim, J', 'say "hi"'], ['Lee', 'ok']])
    expect(parseCsv('a;b\n1;2\n', { rows: 10, columns: 10 }).rows).toEqual([['a', 'b'], ['1', '2']])
    const cut = parseCsv('a,b,c\n1,2,3\n4,5,6\n7,8,9\n', { rows: 2, columns: 2 })
    expect(cut.rows).toEqual([['a', 'b'], ['1', '2']])
    expect(cut.truncated).toBe(true)
    // A formula stays text.
    expect(parseCsv('x\n=1+1\n', { rows: 5, columns: 5 }).rows[1]).toEqual(['=1+1'])
  })
})

describe('upload lifecycle rules', () => {
  it('blocks sending while a file failed or is uploading and lets go once it is removed', async () => {
    transfers.putFile.mockImplementationOnce(() => ({ done: Promise.reject(new Error('upload failed')), abort: vi.fn() }))
    const { result } = renderHook(() => useFileUploads({ workspaceId: 'w1' }), { wrapper: QueryWrapper })
    act(() => result.current.addFiles([file('a.txt'), file('b.txt')]))
    await waitFor(() => expect(result.current.failed).toBe(true))
    expect(result.current.blocking).toBe(true)
    expect(result.current.readyIds).toEqual(['file-2'])
    act(() => result.current.remove(result.current.uploads.find((item) => item.phase === 'failed')!))
    expect(result.current.failed).toBe(false)
    expect(result.current.blocking).toBe(false)
  })

  it('retries a failed transfer', async () => {
    transfers.putFile.mockImplementationOnce(() => ({ done: Promise.reject(new Error('upload failed')), abort: vi.fn() }))
    const { result } = renderHook(() => useFileUploads({ workspaceId: 'w1' }), { wrapper: QueryWrapper })
    act(() => result.current.addFiles([file('a.txt')]))
    await waitFor(() => expect(result.current.failed).toBe(true))
    act(() => result.current.retry(result.current.uploads[0]!))
    await waitFor(() => expect(result.current.readyIds).toHaveLength(1))
    expect(result.current.blocking).toBe(false)
  })

  it('sends at most three files at a time', async () => {
    let live = 0
    let peak = 0
    const releases: Array<() => void> = []
    transfers.putFile.mockImplementation(() => {
      live += 1
      peak = Math.max(peak, live)
      return {
        done: new Promise((resolve) => releases.push(() => { live -= 1; resolve({ state: 'AVAILABLE', mime: 'text/plain' }) })),
        abort: vi.fn(),
      }
    })
    const { result } = renderHook(() => useFileUploads({ workspaceId: 'w1' }), { wrapper: QueryWrapper })
    act(() => result.current.addFiles([file('1'), file('2'), file('3'), file('4'), file('5')]))
    await waitFor(() => expect(releases).toHaveLength(3))
    expect(peak).toBe(3)
    act(() => releases[0]!())
    await waitFor(() => expect(releases).toHaveLength(4))
    expect(peak).toBe(3)
    for (let index = 1; index < 5; index += 1) {
      await waitFor(() => expect(releases[index]).toBeDefined())
      act(() => releases[index]!())
    }
    await waitFor(() => expect(result.current.readyIds).toHaveLength(5))
  })
})
