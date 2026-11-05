import { fireEvent, render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { MessageAttachment, RichTextDoc } from '@penfii/contracts'
import { MediaViewerHost, closeMediaViewer } from '../files/mediaViewer'
import { I18nProvider } from '../../i18n/I18nProvider'
import { LOCALE_STORAGE_KEY } from '../../i18n/messages'
import { MessageBody } from './MessageBody'
import { MessageReference } from './ReplyQuote'

const secret = 'PREVIOUS_SECRET_BODY'
const body: RichTextDoc = {
  type: 'doc',
  content: [{ type: 'paragraph', content: [{ type: 'text', text: secret }] }],
}

describe('deleted message render', () => {
  it('contains no previous body text', () => {
    localStorage.setItem(LOCALE_STORAGE_KEY, 'ko-KR')
    const view = render(
      <I18nProvider>
        <MessageBody message={{ deleted: true, body }} />
        <MessageReference name="Alice" message={{ deleted: true, body }} />
      </I18nProvider>,
    )
    expect(view.container.textContent).not.toContain(secret)
    expect(view.container.querySelectorAll('[data-message-state="deleted"]')).toHaveLength(1)
    expect(view.container.querySelector('.reference.missing')).not.toBeNull()
  })
})

const image: MessageAttachment = {
  id: 'attachment-1',
  fileId: 'file-1',
  filename: 'pixel.png',
  byteSize: 70,
  mime: 'image/png',
  state: 'AVAILABLE',
}

describe('attachment rendering', () => {
  it('uses one attachment content URL and closes the viewer without leaving an overlay', () => {
    localStorage.setItem(LOCALE_STORAGE_KEY, 'ko-KR')
    const view = render(
      <I18nProvider>
        <MediaViewerHost />
        <MessageBody message={{ deleted: false, body: null, attachments: [image, { ...image, id: 'attachment-2', filename: 'notes.txt', mime: 'text/plain' }] }} />
      </I18nProvider>,
    )
    const preview = view.getByRole('img', { name: 'pixel.png' })
    expect(preview.getAttribute('src')).toBe('/v1/attachments/attachment-1/content')
    const links = view.getAllByRole('link', { name: /^다운로드: / })
    expect(links.map((link) => link.getAttribute('href'))).toEqual([
      '/v1/attachments/attachment-1/content',
      '/v1/attachments/attachment-2/content',
    ])
    fireEvent.click(view.getByRole('button', { name: /이미지 확대/ }))
    expect(document.querySelectorAll('.image-viewer')).toHaveLength(1)
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(document.querySelector('.image-viewer')).toBeNull()
    fireEvent.click(view.getByRole('button', { name: /이미지 확대/ }))
    fireEvent.mouseDown(document.querySelector('.image-viewer') as Element)
    expect(document.querySelector('.image-viewer')).toBeNull()
    closeMediaViewer()
  })

  it('does not render an image for a file that is not available', () => {
    localStorage.setItem(LOCALE_STORAGE_KEY, 'ko-KR')
    const view = render(
      <I18nProvider>
        <MessageBody message={{ deleted: false, body: null, attachments: [{ ...image, state: 'QUARANTINED' }, { ...image, id: 'bad', state: 'REJECTED', filename: 'bad.png' }] }} />
      </I18nProvider>,
    )
    expect(view.container.querySelector('img')).toBeNull()
    expect(view.container.textContent).toContain('확인 중')
    expect(view.container.textContent).toContain('사용할 수 없는 파일')
  })
})
