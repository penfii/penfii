import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import type { ReactElement } from 'react'
import { RouterProvider, createMemoryRouter } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '@penfii/api-client'
import type { DocNode, DocPage, DocVersion } from '@penfii/contracts'

const api = vi.hoisted(() => ({
  getDocTree: vi.fn(), createDoc: vi.fn(), updateDoc: vi.fn(), moveDoc: vi.fn(), deleteDoc: vi.fn(), getDoc: vi.fn(),
  getDocComments: vi.fn(), createDocComment: vi.fn(), editDocComment: vi.fn(), deleteDocComment: vi.fn(),
  saveDoc: vi.fn(), createDocVersion: vi.fn(), getDocVersions: vi.fn(), getDocVersion: vi.fn(),
  getDocParticipants: vi.fn(), searchDirectory: vi.fn(), resolveReferences: vi.fn(), getDocTemplates: vi.fn(), getDocReferences: vi.fn(), me: vi.fn(),
  getSpace: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

// The real editor (BlockNote) is loaded lazily and needs a real browser; the page is tested around it.
// The stand-in registers a controller like the real one and lets a test say "the draft changed".
const editor = vi.hoisted(() => ({ content: { schema: 'penfii.doc', version: 1, blocks: ['draft'] } as unknown }))
vi.mock('./DocEditor', async () => {
  const React = await import('react')
  return {
    default: function DocEditor({ editable, content, onController, onDirtyChange }: { editable: boolean; content: unknown; onController?: (c: unknown) => void; onDirtyChange?: (d: boolean) => void }) {
      React.useEffect(() => {
        onController?.({ getContent: () => editor.content, isDirty: () => false })
        return () => onController?.(null)
      }, [onController])
      return (
        <div data-testid="editor" data-editable={String(editable)} data-content={JSON.stringify(content)}>
          <button type="button" onClick={() => onDirtyChange?.(true)}>type</button>
        </div>
      )
    },
  }
})

import { DocTree } from './DocTree'
import { DocPageView } from './DocPageView'
import { renderWithProviders } from '../../test/renderWithProviders'

const node = (id: string, parentId: string | null, title = id): DocNode => ({
  id, parentId, title, position: 1, visibility: 'WORKSPACE', projectId: null, updatedAt: '2026-10-23T00:00:00Z',
  rights: { edit: true, manage: true },
})
const page = (over: Partial<DocPage> = {}): DocPage => ({
  id: 'p1', workspaceId: 'w', projectId: null, parentId: null, title: '회의록', visibility: 'WORKSPACE', scopeProjectId: null, scopeTeamId: null, position: 1,
  content: { schema: 'penfii.doc', version: 1, blocks: [] }, revisionNo: 3, latestVersionNo: 2, hasUnversionedChanges: true, materializedAt: '', updatedAt: '', updatedBy: null, createdBy: 'u1',
  liveEditing: false, myRole: 'MANAGER', rights: { edit: true, manage: true }, ...over,
})
const version = (over: Partial<DocVersion> = {}): DocVersion => ({
  versionNo: 1, title: '첫 제목', sourceRevisionNo: 1, createdBy: 'u1', createdByName: 'Alice', createdAt: '2026-10-20T00:00:00Z', isCurrent: false,
  content: { schema: 'penfii.doc', version: 1, blocks: ['old'] }, ...over,
})

function mount(ui: ReactElement, path = '/w/w/docs') {
  return renderWithProviders(ui, { route: path, path: '/w/:workspaceId/docs' })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'ko-KR')
  editor.content = { schema: 'penfii.doc', version: 1, blocks: ['draft'] }
  for (const fn of Object.values(api)) fn.mockReset()
  api.getDocTree.mockResolvedValue([node('a', null, '루트'), node('b', 'a', '하위')])
  api.getDocComments.mockResolvedValue({ items: [], nextCursor: null })
  api.getDocParticipants.mockResolvedValue([])
  api.getDocTemplates.mockResolvedValue([])
  api.resolveReferences.mockResolvedValue([])
  api.getDocReferences.mockResolvedValue({ items: [], nextCursor: null })
  api.me.mockResolvedValue({ id: 'u1', email: 'a@x', displayName: 'Alice' })
  api.getSpace.mockResolvedValue({ teams: [], projects: [], channels: [], dms: [] })
})
afterEach(cleanup)

describe('Doc tree', () => {
  it('shows pages nested under their parents and opens the path to the selected page', async () => {
    mount(<DocTree workspaceId="w" selectedId="b" onSelect={() => undefined} />)
    expect(await screen.findByRole('treeitem', { name: /하위/ })).toBeTruthy()
    expect(screen.getByRole('treeitem', { name: /루트/ })).toBeTruthy()
  })

  it('asks for a title before creating a page, never creates one without it, then selects it', async () => {
    api.createDoc.mockResolvedValue(page({ id: 'new' }))
    const selected: Array<string | null> = []
    mount(<DocTree workspaceId="w" selectedId={null} onSelect={(id) => selected.push(id)} />)
    fireEvent.click(await screen.findByRole('button', { name: /새 문서/ }))
    const create = await screen.findByRole('button', { name: '만들기' })
    expect((create as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(screen.getByLabelText('제목'), { target: { value: '   ' } })
    expect((create as HTMLButtonElement).disabled).toBe(true)
    expect(api.createDoc).not.toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText('제목'), { target: { value: ' 주간 회의 ' } })
    fireEvent.click(create)
    await waitFor(() => expect(selected).toEqual(['new']))
    expect(api.createDoc).toHaveBeenCalledWith('w', expect.objectContaining({ title: '주간 회의', visibility: 'WORKSPACE' }))

    api.moveDoc.mockRejectedValue(new ApiError(422, 'doc_cycle', 'cycle'))
    fireEvent.click(screen.getByRole('button', { name: /이동: 루트/ }))
    // The picker lists only pages the move may go under; the server's refusal is still shown.
    fireEvent.click(await screen.findByRole('button', { name: '이동', hidden: true }).catch(() => screen.getAllByRole('button', { name: '이동' }).at(-1)!))
    await waitFor(() => expect(api.moveDoc).toHaveBeenCalled())
    expect(await screen.findByText('페이지를 자기 자신이나 하위 페이지 아래로 옮길 수 없습니다.')).toBeTruthy()
  })

  it('asks before deleting and deletes the page with its subpages', async () => {
    api.deleteDoc.mockResolvedValue(undefined)
    mount(<DocTree workspaceId="w" selectedId={null} onSelect={() => undefined} />)
    fireEvent.click(await screen.findByRole('button', { name: /삭제: 루트/ }))
    expect(await screen.findByText(/와 그 하위 페이지를 삭제할까요/)).toBeTruthy()
    expect(api.deleteDoc).not.toHaveBeenCalled()
    fireEvent.click(screen.getAllByRole('button', { name: '삭제' }).at(-1)!)
    await waitFor(() => expect(api.deleteDoc).toHaveBeenCalledWith('a'))
  })
})

describe('Doc tree actions follow the rights of each page', () => {
  it('offers move to an editor, delete to a manager, and nothing to a reader', async () => {
    api.getDocTree.mockResolvedValue([
      { ...node('a', null, '관리'), rights: { edit: true, manage: true } },
      { ...node('b', null, '편집'), rights: { edit: true, manage: false } },
      { ...node('c', null, '읽기'), rights: { edit: false, manage: false } },
    ])
    mount(<DocTree workspaceId="w" selectedId={null} onSelect={() => undefined} />)
    await screen.findByRole('treeitem', { name: /관리/ })
    const actions = (title: string) => ({
      move: screen.queryByRole('button', { name: `이동: ${title}` }) !== null,
      remove: screen.queryByRole('button', { name: `삭제: ${title}` }) !== null,
    })
    expect([actions('관리').move, actions('관리').remove]).toEqual([true, true])
    expect([actions('편집').move, actions('편집').remove]).toEqual([true, false])
    expect([actions('읽기').move, actions('읽기').remove]).toEqual([false, false])
    // A reader cannot start a rename or a delete from the keyboard either.
    const reader = screen.getByRole('treeitem', { name: /읽기/ })
    fireEvent.focus(reader)
    fireEvent.keyDown(reader, { key: 'F2' })
    expect(screen.queryByLabelText('이름 바꾸기')).toBeNull()
    fireEvent.keyDown(reader, { key: 'Delete' })
    expect(screen.queryByText(/“읽기”/)).toBeNull()
  })
})

describe('Doc page', () => {
  it('shows the unavailable state for a page that is deleted or not allowed, without a title', async () => {
    api.getDoc.mockRejectedValue(new ApiError(404, 'not_found', 'not found'))
    mount(<DocPageView workspaceId="w" docId="p1" />)
    expect(await screen.findByText('열 수 없는 페이지')).toBeTruthy()
    expect(screen.queryByTestId('editor')).toBeNull()
  })

  it('reads first: Edit, Create Version and History, an editor that cannot be typed in, and no Save or Cancel', async () => {
    api.getDoc.mockResolvedValue(page())
    mount(<DocPageView workspaceId="w" docId="p1" />)
    expect(await screen.findByRole('button', { name: '편집' })).toBeTruthy()
    expect(screen.getByRole('button', { name: '버전 만들기' })).toBeTruthy()
    expect(screen.getByRole('button', { name: '기록' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: '저장' })).toBeNull()
    expect((await screen.findByTestId('editor')).dataset.editable).toBe('false')
    expect(screen.getByText(/최신 버전 2/)).toBeTruthy()
  })

  it('edits as a draft: Save and Cancel replace Edit, and nothing is sent while editing', async () => {
    api.getDoc.mockResolvedValue(page())
    mount(<DocPageView workspaceId="w" docId="p1" />)
    fireEvent.click(await screen.findByRole('button', { name: '편집' }))
    expect(await screen.findByRole('button', { name: '저장' })).toBeTruthy()
    expect(screen.getByRole('button', { name: '취소' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: '편집' })).toBeNull()
    expect(screen.queryByRole('button', { name: '버전 만들기' })).toBeNull()
    expect((await screen.findByTestId('editor')).dataset.editable).toBe('true')
    fireEvent.change(screen.getByLabelText('문서 제목'), { target: { value: '새 제목' } })
    fireEvent.click(screen.getByText('type'))
    expect(api.saveDoc).not.toHaveBeenCalled()
    expect(api.updateDoc).not.toHaveBeenCalled()
    expect(api.createDocVersion).not.toHaveBeenCalled()
  })

  it('Save sends the draft with the revision it started from, creates no Version, and returns to reading', async () => {
    api.getDoc.mockResolvedValue(page())
    api.saveDoc.mockResolvedValue(page({ title: '새 제목', revisionNo: 4 }))
    mount(<DocPageView workspaceId="w" docId="p1" />)
    fireEvent.click(await screen.findByRole('button', { name: '편집' }))
    fireEvent.change(await screen.findByLabelText('문서 제목'), { target: { value: '  새 제목 ' } })
    fireEvent.click(screen.getByRole('button', { name: '저장' }))
    await waitFor(() => expect(api.saveDoc).toHaveBeenCalledWith('p1', { expectedRevisionNo: 3, title: '새 제목', content: editor.content }))
    expect(api.createDocVersion).not.toHaveBeenCalled()
    expect(await screen.findByRole('button', { name: '편집' })).toBeTruthy()
    expect(screen.getByText('저장했습니다')).toBeTruthy()
  })

  it('cannot save without a title', async () => {
    api.getDoc.mockResolvedValue(page({ title: '' }))
    mount(<DocPageView workspaceId="w" docId="p1" />)
    fireEvent.click(await screen.findByRole('button', { name: '편집' }))
    const save = await screen.findByRole('button', { name: '저장' })
    expect((save as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(screen.getByLabelText('문서 제목'), { target: { value: '   ' } })
    expect((save as HTMLButtonElement).disabled).toBe(true)
    expect(api.saveDoc).not.toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText('문서 제목'), { target: { value: '이름' } })
    expect((save as HTMLButtonElement).disabled).toBe(false)
  })

  it('keeps the draft when someone saved first, and says so; loading the latest drops it', async () => {
    api.getDoc.mockResolvedValue(page())
    api.saveDoc.mockRejectedValue(new ApiError(409, 'version_conflict', 'conflict'))
    mount(<DocPageView workspaceId="w" docId="p1" />)
    fireEvent.click(await screen.findByRole('button', { name: '편집' }))
    fireEvent.click(await screen.findByRole('button', { name: '저장' }))
    expect(await screen.findByText(/다른 곳에서 먼저 저장되어/)).toBeTruthy()
    expect(screen.getByRole('button', { name: '저장' })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '최신 내용 불러오기' }))
    expect(await screen.findByRole('button', { name: '편집' })).toBeTruthy()
  })

  it('Cancel leaves at once when nothing changed and asks before throwing a draft away', async () => {
    api.getDoc.mockResolvedValue(page())
    mount(<DocPageView workspaceId="w" docId="p1" />)
    fireEvent.click(await screen.findByRole('button', { name: '편집' }))
    fireEvent.click(await screen.findByRole('button', { name: '취소' }))
    expect(await screen.findByRole('button', { name: '편집' })).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: '편집' }))
    await screen.findByRole('button', { name: '저장' })
    fireEvent.click(screen.getByText('type'))
    fireEvent.click(screen.getByRole('button', { name: '취소' }))
    fireEvent.click(await screen.findByRole('button', { name: '버리기' }))
    expect(await screen.findByRole('button', { name: '편집' })).toBeTruthy()
    expect(api.saveDoc).not.toHaveBeenCalled()
  })

  it('asks before any navigation leaves an unsaved draft', async () => {
    api.getDoc.mockResolvedValue(page())
    api.saveDoc.mockResolvedValue(page({ revisionNo: 4 }))
    const router = createMemoryRouter(
      [
        { path: '/w/:workspaceId/docs', element: <DocPageView workspaceId="w" docId="p1" /> },
        { path: '/elsewhere', element: <p>다른 곳</p> },
      ],
      { initialEntries: ['/w/w/docs?doc=p1'] },
    )
    renderWithProviders(<RouterProvider router={router} />)
    fireEvent.click(await screen.findByRole('button', { name: '편집' }))
    await screen.findByRole('button', { name: '저장' })
    fireEvent.click(screen.getByText('type'))
    // A change: another page, another section, and back are all held until the person decides.
    void router.navigate('/w/w/docs?doc=p2')
    fireEvent.click(await screen.findByRole('button', { name: '계속 편집' }))
    await waitFor(() => expect(router.state.location.search).toBe('?doc=p1'))
    void router.navigate('/elsewhere')
    fireEvent.click(await screen.findByRole('button', { name: '버리고 이동' }))
    await waitFor(() => expect(router.state.location.pathname).toBe('/elsewhere'))
    expect(api.saveDoc).not.toHaveBeenCalled()
  })

  it('does not hold navigation after the draft was saved', async () => {
    api.getDoc.mockResolvedValue(page())
    api.saveDoc.mockResolvedValue(page({ revisionNo: 4 }))
    const router = createMemoryRouter(
      [
        { path: '/w/:workspaceId/docs', element: <DocPageView workspaceId="w" docId="p1" /> },
        { path: '/elsewhere', element: <p>다른 곳</p> },
      ],
      { initialEntries: ['/w/w/docs?doc=p1'] },
    )
    renderWithProviders(<RouterProvider router={router} />)
    fireEvent.click(await screen.findByRole('button', { name: '편집' }))
    await screen.findByRole('button', { name: '저장' })
    fireEvent.click(screen.getByText('type'))
    fireEvent.click(screen.getByRole('button', { name: '저장' }))
    await screen.findByRole('button', { name: '편집' })
    await router.navigate('/elsewhere')
    expect(router.state.location.pathname).toBe('/elsewhere')
  })

  it('shows a notice for a page without a title and keeps Create Version off until it has one', async () => {
    api.getDoc.mockResolvedValue(page({ title: '', hasUnversionedChanges: true }))
    mount(<DocPageView workspaceId="w" docId="p1" />)
    expect(await screen.findByText(/이 문서에는 제목이 없습니다/)).toBeTruthy()
    expect((screen.getByRole('button', { name: '버전 만들기' }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: '편집' }))
    const title = await screen.findByLabelText('문서 제목')
    expect(document.activeElement).toBe(title)
  })

  it('Create Version freezes the saved page with its revision; an unchanged page is refused and disabled', async () => {
    api.getDoc.mockResolvedValue(page())
    api.createDocVersion.mockResolvedValue({ versionNo: 3 })
    mount(<DocPageView workspaceId="w" docId="p1" />)
    fireEvent.click(await screen.findByRole('button', { name: '버전 만들기' }))
    await waitFor(() => expect(api.createDocVersion).toHaveBeenCalledWith('p1', 3))
    expect(await screen.findByText('버전을 만들었습니다 (버전 3)')).toBeTruthy()
    expect(api.saveDoc).not.toHaveBeenCalled()

    cleanup()
    api.getDoc.mockResolvedValue(page({ hasUnversionedChanges: false }))
    mount(<DocPageView workspaceId="w" docId="p1" />)
    const button = await screen.findByRole('button', { name: '버전 만들기' })
    expect((button as HTMLButtonElement).disabled).toBe(true)
  })

  it('says so when the server finds nothing new to freeze', async () => {
    api.getDoc.mockResolvedValue(page())
    api.createDocVersion.mockRejectedValue(new ApiError(422, 'doc_version_unchanged', 'same'))
    mount(<DocPageView workspaceId="w" docId="p1" />)
    fireEvent.click(await screen.findByRole('button', { name: '버전 만들기' }))
    expect(await screen.findByText('마지막 버전 이후 저장된 변경이 없습니다')).toBeTruthy()
  })

  it('History lists only created Versions; Restore loads one into a draft and changes nothing until Save', async () => {
    api.getDoc.mockResolvedValue(page())
    api.getDocVersions.mockResolvedValue({ items: [{ ...version({ versionNo: 2, title: '둘째' }), isCurrent: false }, version()], nextBefore: null })
    api.getDocVersion.mockResolvedValue(version())
    api.saveDoc.mockResolvedValue(page({ title: '첫 제목', revisionNo: 4 }))
    mount(<DocPageView workspaceId="w" docId="p1" />)
    fireEvent.click(await screen.findByRole('button', { name: '기록' }))
    expect(await screen.findByText('버전 2')).toBeTruthy()
    expect(screen.getByText('버전 1')).toBeTruthy()
    fireEvent.click(screen.getAllByRole('button', { name: '복원' }).at(-1)!)
    await waitFor(() => expect(api.getDocVersion).toHaveBeenCalledWith('p1', 1))
    expect((await screen.findByLabelText('문서 제목') as HTMLInputElement).value).toBe('첫 제목')
    expect(JSON.parse((await screen.findByTestId('editor')).dataset.content!)).toEqual({ schema: 'penfii.doc', version: 1, blocks: ['old'] })
    expect(screen.getByText(/버전 1의 내용을 가져왔습니다/)).toBeTruthy()
    expect(api.saveDoc).not.toHaveBeenCalled()
    expect(api.createDocVersion).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: '저장' }))
    await waitFor(() => expect(api.saveDoc).toHaveBeenCalledWith('p1', expect.objectContaining({ expectedRevisionNo: 3, title: '첫 제목' })))
  })

  it('posts a comment with the mentioned person as a mention node', async () => {
    api.getDoc.mockResolvedValue(page())
    api.searchDirectory.mockResolvedValue({ query: 'b', results: [{ type: 'person', id: 'u2', title: 'Bob Lee', subtitle: '', person: { id: 'u2', displayName: 'Bob Lee', username: null, email: '', organizationId: 'o', organizationName: 'Acme', affiliation: [] } }] })
    api.createDocComment.mockResolvedValue({})
    mount(<DocPageView workspaceId="w" docId="p1" />)
    const box = await screen.findByPlaceholderText(/댓글을 입력하세요/)
    fireEvent.change(box, { target: { value: '@Bo', selectionStart: 3 } })
    ;(box as HTMLTextAreaElement).setSelectionRange(3, 3)
    fireEvent.change(box, { target: { value: '@Bo' } })
    fireEvent.click(await screen.findByRole('button', { name: 'Bob Lee' }))
    fireEvent.change(box, { target: { value: '@Bob Lee 확인 부탁' } })
    fireEvent.click(screen.getByRole('button', { name: '댓글 달기' }))
    await waitFor(() => expect(api.createDocComment).toHaveBeenCalled())
    const body = api.createDocComment.mock.calls[0]![1] as { content: Array<{ content: Array<{ type: string; attrs?: { userId: string } }> }> }
    expect(body.content[0]!.content.some((c) => c.type === 'mention' && c.attrs?.userId === 'u2')).toBe(true)
  })

  it('edit view offers only Save and Cancel (History and Restore live in the read view)', async () => {
    api.getDoc.mockResolvedValue(page())
    mount(<DocPageView workspaceId="w" docId="p1" />)
    fireEvent.click(await screen.findByRole('button', { name: '편집' }))
    await screen.findByRole('button', { name: '저장' })
    expect(screen.queryByRole('button', { name: '기록' })).toBeNull()
    expect(screen.getByRole('button', { name: '취소' })).toBeTruthy()
    expect(screen.getByText('변경 사항 없음')).toBeTruthy()
  })

  it('says why Save is off while the title is empty, and turns it on as soon as one is typed', async () => {
    api.getDoc.mockResolvedValue(page({ title: '' }))
    mount(<DocPageView workspaceId="w" docId="p1" />)
    fireEvent.click(await screen.findByRole('button', { name: '편집' }))
    const save = (await screen.findByRole('button', { name: '저장' })) as HTMLButtonElement
    expect(save.disabled).toBe(true)
    expect(screen.getByText('제목을 입력하면 저장할 수 있습니다')).toBeTruthy()
    expect(save.title).toBe('제목을 입력하면 저장할 수 있습니다')
    fireEvent.click(screen.getByText('type'))
    expect(save.disabled).toBe(true)
    fireEvent.change(screen.getByLabelText('문서 제목'), { target: { value: 'ㄱ' } })
    expect(save.disabled).toBe(false)
    expect(screen.getByText('저장하지 않은 변경')).toBeTruthy()
  })

  it('offers no Edit or Create Version to someone who may only read, but shows the page and History', async () => {
    api.getDoc.mockResolvedValue(page({ rights: { edit: false, manage: false }, myRole: null }))
    mount(<DocPageView workspaceId="w" docId="p1" />)
    expect(await screen.findByText('이 문서는 볼 수만 있습니다.')).toBeTruthy()
    expect(screen.queryByRole('button', { name: '편집' })).toBeNull()
    expect(screen.queryByRole('button', { name: '버전 만들기' })).toBeNull()
    expect(screen.getByRole('button', { name: '기록' })).toBeTruthy()
    expect((await screen.findByTestId('editor')).dataset.editable).toBe('false')
    // Comments are not an edit right: the same person can comment.
    expect(screen.getByPlaceholderText('댓글을 입력하세요 (@로 사람을 언급)')).toBeTruthy()
  })
})
