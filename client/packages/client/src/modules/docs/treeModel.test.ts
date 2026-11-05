import { describe, expect, it } from 'vitest'
import type { DocNode } from '@penfii/contracts'
import { ancestorIds, buildTree, canMoveUnder, descendantIds, flatten, groupByProject, matchingDocs } from './treeModel'

const node = (id: string, parentId: string | null, position = 0): DocNode => ({
  id, parentId, title: id, position, visibility: 'WORKSPACE', projectId: null, updatedAt: '2026-10-23T00:00:00Z',
  rights: { edit: true, manage: true },
})

const nodes = [node('a', null, 1), node('b', 'a', 1), node('c', 'b', 1), node('d', null, 2), node('orphan', 'hidden-parent', 3)]

describe('doc tree model', () => {
  it('nests pages and puts a page with an unknown parent at the top', () => {
    const roots = buildTree(nodes)
    expect(roots.map((r) => r.id)).toEqual(['a', 'd', 'orphan'])
    expect(roots[0]!.children[0]!.children[0]!.id).toBe('c')
  })

  it('refuses to move a page into itself or its own subtree', () => {
    expect(canMoveUnder(nodes, 'a', 'a')).toBe(false)
    expect(canMoveUnder(nodes, 'a', 'c')).toBe(false)
    expect(canMoveUnder(nodes, 'a', 'd')).toBe(true)
    expect(canMoveUnder(nodes, 'c', null)).toBe(true)
    expect([...descendantIds(nodes, 'a')].sort()).toEqual(['b', 'c'])
  })

  it('lists only the rows whose parents are open, and finds the path to a page', () => {
    const roots = buildTree(nodes)
    expect(flatten(roots, new Set()).map((r) => r.node.id)).toEqual(['a', 'd', 'orphan'])
    const open = flatten(roots, new Set(['a', 'b']))
    expect(open.map((r) => [r.node.id, r.depth])).toEqual([['a', 0], ['b', 1], ['c', 2], ['d', 0], ['orphan', 0]])
    expect(ancestorIds(nodes, 'c')).toEqual(['b', 'a'])
  })

  it('groups root pages by project, participating ones first, and keeps a child with its parent', () => {
    const pages = [
      { ...node('none', null), title: 'Loose' },
      { ...node('p2', null), projectId: 'beta', title: 'Beta root' },
      { ...node('p2c', 'p2'), projectId: 'alpha', title: 'Nested elsewhere' },
      { ...node('p1', null), projectId: 'alpha', title: 'Alpha root' },
    ]
    const groups = groupByProject(buildTree(pages), ['alpha', 'beta'])
    expect(groups.map((group) => [group.key, group.count, group.roots.map((root) => root.id)])).toEqual([
      ['alpha', 1, ['p1']],
      ['beta', 2, ['p2']],
      ['none', 1, ['none']],
    ])
    expect(matchingDocs(pages, 'nested').map((page) => page.id).sort()).toEqual(['p2', 'p2c'])
  })
})
