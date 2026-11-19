import { act, cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { useState } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '@penfii/api-client'
import type { DocNode } from '@penfii/contracts'
import { deleteScope } from './treeModel'

const api = vi.hoisted(() => ({ getDocTree: vi.fn(), deleteDoc: vi.fn(), getSpace: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { DocTree } from './DocTree'
import { DeleteDocDialog } from './DeleteDocDialog'
import { NoticeHost, dismissNotice } from '../../design/notices'
import { renderWithProviders } from '../../test/renderWithProviders'

const doc = (id: string, parentId: string | null, title = id, manage = true): DocNode => ({
  id, parentId, title, position: 1, visibility: 'WORKSPACE', projectId: null, updatedAt: '2026-10-23T00:00:00Z', rights: { edit: manage, manage },
})

describe('what deleting a page does (the server keeps what cannot be managed)', () => {
  it('deletes the page alone when nothing is below it', () => {
    const scope = deleteScope([doc('a', null), doc('b', null)], 'a')!
    expect(scope.deleted.map((n) => n.id)).toEqual(['a'])
    expect(scope.moved).toEqual([])
  })

  it('deletes manageable subpages at any depth', () => {
    const scope = deleteScope([doc('a', null), doc('b', 'a'), doc('c', 'b'), doc('d', null)], 'a')!
    expect(scope.deleted.map((n) => n.id)).toEqual(['a', 'b', 'c'])
  })

  it('keeps a subpage that cannot be managed, with everything below it, and moves it to the deleted page\'s parent', () => {
    const nodes = [doc('top', null, 'Top'), doc('a', 'top'), doc('keep', 'a', 'Keep', false), doc('under', 'keep'), doc('gone', 'a')]
    const scope = deleteScope(nodes, 'a')!
    expect(scope.deleted.map((n) => n.id)).toEqual(['a', 'gone'])
    expect(scope.moved.map((n) => n.id)).toEqual(['keep'])
    expect(scope.destination?.id).toBe('top')
  })

  it('moves to the top when the page has no visible parent', () => {
    const scope = deleteScope([doc('a', null), doc('keep', 'a', 'Keep', false)], 'a')!
    expect(scope.destination).toBeNull()
  })

  it('knows nothing about a page that is not in the list', () => {
    expect(deleteScope([doc('a', null)], 'zzz')).toBeNull()
  })
})

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'ko-KR')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getSpace.mockResolvedValue({ teams: [], projects: [], channels: [], dms: [] })
  api.deleteDoc.mockResolvedValue(undefined)
})
afterEach(() => { act(() => dismissNotice()); cleanup() })

let tree: DocNode[] = []
let gate: Promise<DocNode[]> | null = null
let failing: ApiError | null = null

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'ko-KR')
  Object.values(api).forEach((fn) => fn.mockReset())
  gate = null
  failing = null
  api.getSpace.mockResolvedValue({ teams: [], projects: [], channels: [], dms: [] })
  api.getDocTree.mockImplementation(() => (failing ? Promise.reject(failing) : (gate ?? Promise.resolve(tree))))
  api.deleteDoc.mockResolvedValue(undefined)
})
afterEach(() => { act(() => dismissNotice()); cleanup() })

function mount(props: { selectedId?: string | null; onSelect?: () => void; onGone?: () => void } = {}) {
  return renderWithProviders(
    <><DocTree workspaceId="w" selectedId={props.selectedId ?? null} onSelect={props.onSelect ?? (() => undefined)} onGone={props.onGone} /><NoticeHost /></>,
    { route: '/w/w/docs', path: '/w/:workspaceId/docs' },
  )
}
const ask = async (title: string) => fireEvent.click(await screen.findByRole('button', { name: `삭제: ${title}` }))
const dialog = () => screen.getByRole('dialog')
const deleteButton = () => within(dialog()).getByRole('button', { name: '삭제' }) as HTMLButtonElement

describe('the delete question', () => {
  it('has a danger Delete, the first focus on Cancel, and says the page cannot be restored', async () => {
    tree = [doc('a', null, '회의록')]
    mount()
    await ask('회의록')
    expect(await screen.findByText('“회의록” 1개를 삭제합니다.')).toBeTruthy()
    expect(deleteButton().className).toContain('btn-danger')
    expect(document.activeElement).toBe(within(dialog()).getByRole('button', { name: '취소' }))
    expect(screen.getByText('삭제한 문서는 지금 제품에서 되살릴 수 없습니다.')).toBeTruthy()
  })

  it('does not allow Delete while checking', async () => {
    tree = [doc('a', null, '회의록')]
    mount()
    await screen.findByText('회의록')
    let answer: (nodes: DocNode[]) => void = () => undefined
    gate = new Promise((resolve) => { answer = resolve })
    await ask('회의록')
    expect(await screen.findByText('삭제 범위를 확인하는 중…')).toBeTruthy()
    expect(deleteButton().disabled).toBe(true)
    gate = null
    await act(async () => answer(tree))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    expect(api.deleteDoc).not.toHaveBeenCalled()
  })

  it('does not allow Delete when the check fails, until Check again works', async () => {
    tree = [doc('a', null, '회의록')]
    mount()
    await screen.findByText('회의록')
    failing = new ApiError(502, 'error', 'HTTP 502')
    await ask('회의록')
    expect(await screen.findByText('삭제 범위를 확인하지 못했습니다')).toBeTruthy()
    expect(deleteButton().disabled).toBe(true)
    failing = null
    fireEvent.click(within(dialog()).getByRole('button', { name: '다시 확인' }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    expect(screen.getByText('“회의록” 1개를 삭제합니다.')).toBeTruthy()
    expect(api.deleteDoc).not.toHaveBeenCalled()
  })

  it('names the subpages and the ones that stay, and says nothing of pages the person cannot see', async () => {
    tree = [doc('top', null, '상위'), doc('a', 'top', '기획'), doc('b', 'a', '일정'), doc('k', 'a', '보안 검토', false)]
    mount({ selectedId: 'a' })
    await ask('기획')
    expect(await screen.findByText('“기획”와(과) 하위 문서 1개를 삭제합니다.')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '하위 문서 1개 보기' }))
    expect(within(dialog()).getByText('일정')).toBeTruthy()
    expect(within(dialog()).getByText('“보안 검토”은(는) 삭제 권한이 없어 남고, “상위” 아래로 옮겨집니다.')).toBeTruthy()
  })

  it('tells when the structure changed after the check and sends nothing until asked again', async () => {
    tree = [doc('a', null, '기획'), doc('b', 'a', '일정')]
    mount()
    await ask('기획')
    await screen.findByText('“기획”와(과) 하위 문서 1개를 삭제합니다.')
    tree = [doc('a', null, '기획'), doc('b', 'a', '일정'), doc('c', 'a', '새 문서')]
    fireEvent.click(deleteButton())
    expect(await screen.findByText(/확인한 뒤 문서 구조가 바뀌었습니다/)).toBeTruthy()
    expect(screen.getByText('“기획”와(과) 하위 문서 2개를 삭제합니다.')).toBeTruthy()
    expect(api.deleteDoc).not.toHaveBeenCalled()
    fireEvent.click(deleteButton())
    await waitFor(() => expect(api.deleteDoc).toHaveBeenCalledWith('a'))
  })

  it('keeps the question open and offers Try again when deleting fails, saying the page is still there', async () => {
    tree = [doc('a', null, '기획')]
    api.deleteDoc.mockRejectedValueOnce(new ApiError(502, 'error', 'HTTP 502'))
    mount()
    await ask('기획')
    await screen.findByText('“기획” 1개를 삭제합니다.')
    fireEvent.click(deleteButton())
    expect(await screen.findByText('문서를 삭제하지 못했습니다. 문서는 그대로 있습니다.')).toBeTruthy()
    expect(screen.queryByText(/을 삭제했습니다/)).toBeNull()
    fireEvent.click(within(dialog()).getByRole('button', { name: '다시 시도' }))
    await waitFor(() => expect(api.deleteDoc).toHaveBeenCalledTimes(2))
  })

  it('offers no retry for a permission failure', async () => {
    tree = [doc('a', null, '기획')]
    api.deleteDoc.mockRejectedValueOnce(new ApiError(403, 'forbidden', 'forbidden'))
    mount()
    await ask('기획')
    await screen.findByText('“기획” 1개를 삭제합니다.')
    fireEvent.click(deleteButton())
    expect(await screen.findByText('이 문서를 삭제할 권한이 없습니다.')).toBeTruthy()
    expect(within(dialog()).queryByRole('button', { name: '다시 시도' })).toBeNull()
  })

  it('says what was really deleted, then moves focus to the next row', async () => {
    tree = [doc('a', null, '첫째'), doc('b', null, '둘째'), doc('c', null, '셋째')]
    mount()
    await ask('둘째')
    await screen.findByText('“둘째” 1개를 삭제합니다.')
    api.deleteDoc.mockImplementation(() => {
      tree = [tree[0], tree[2]]
      return Promise.resolve(undefined)
    })
    fireEvent.click(deleteButton())
    expect(await screen.findByText('“둘째”을(를) 삭제했습니다.')).toBeTruthy()
    await waitFor(() => expect(document.activeElement?.id).toBe('doc-row-c'))
  })

  it('counts the subpages that really went', async () => {
    tree = [doc('a', null, '기획'), doc('b', 'a', '일정')]
    mount()
    await ask('기획')
    await screen.findByText('“기획”와(과) 하위 문서 1개를 삭제합니다.')
    api.deleteDoc.mockImplementation(() => {
      tree = []
      return Promise.resolve(undefined)
    })
    fireEvent.click(deleteButton())
    expect(await screen.findByText('“기획”와(과) 하위 문서 1개를 삭제했습니다.')).toBeTruthy()
  })

  it('warns when the result differs from what was shown', async () => {
    tree = [doc('a', null, '기획'), doc('b', 'a', '일정')]
    mount()
    await ask('기획')
    await screen.findByText('“기획”와(과) 하위 문서 1개를 삭제합니다.')
    api.deleteDoc.mockImplementation(() => {
      tree = [doc('b', null, '일정')]
      return Promise.resolve(undefined)
    })
    fireEvent.click(deleteButton())
    expect(await screen.findByText(/확인한 범위와 달리 문서 1개가 삭제되었습니다/)).toBeTruthy()
  })

  it('closes and tells when the page was already deleted, without sending a delete', async () => {
    tree = [doc('a', null, '기획')]
    mount()
    await ask('기획')
    await screen.findByText('“기획” 1개를 삭제합니다.')
    tree = []
    fireEvent.click(deleteButton())
    expect(await screen.findByText('이미 삭제된 문서입니다.')).toBeTruthy()
    expect(api.deleteDoc).not.toHaveBeenCalled()
  })

  it('leaves the page being read when that page is deleted with its parent', async () => {
    tree = [doc('a', null, '기획'), doc('b', 'a', '일정')]
    const onSelect = vi.fn()
    const onGone = vi.fn()
    mount({ selectedId: 'b', onSelect, onGone })
    await ask('기획')
    await screen.findByText('“기획”와(과) 하위 문서 1개를 삭제합니다.')
    api.deleteDoc.mockImplementation(() => {
      tree = []
      return Promise.resolve(undefined)
    })
    fireEvent.click(deleteButton())
    await waitFor(() => expect(onGone).toHaveBeenCalled())
    expect(onSelect).toHaveBeenCalledWith(null)
  })
})

describe('focus after the question', () => {
  it('returns to the row when the question is cancelled', async () => {
    tree = [doc('a', null, '기획')]
    mount()
    await ask('기획')
    await screen.findByText('“기획” 1개를 삭제합니다.')
    fireEvent.click(within(dialog()).getByRole('button', { name: '취소' }))
    await waitFor(() => expect(document.activeElement?.id).toBe('doc-row-a'))
  })
})

describe('the question while the tree behind it re-renders', () => {
  function Host({ node }: { node: DocNode }) {
    const [count, setCount] = useState(0)
    // A new onClose on every render, as the tree gives it.
    return (
      <>
        <button type="button" onClick={() => setCount(count + 1)}>rerender {count}</button>
        <DeleteDocDialog workspaceId="w" node={node} onClose={() => undefined} onDeleted={() => undefined} />
      </>
    )
  }

  it('reads the tree once for the question, not again on every re-render', async () => {
    tree = [doc('a', null, '기획')]
    renderWithProviders(<Host node={tree[0]} />)
    await screen.findByText('“기획” 1개를 삭제합니다.')
    for (let i = 0; i < 3; i += 1) fireEvent.click(screen.getByRole('button', { name: /^rerender/, hidden: true }))
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 20)) })
    expect(api.getDocTree).toHaveBeenCalledTimes(1)
    expect(screen.queryByText('삭제 범위를 확인하는 중…')).toBeNull()
  })
})
