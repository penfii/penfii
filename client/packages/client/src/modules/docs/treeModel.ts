import type { DocNode } from '@penfii/contracts'

export type TreeNode = DocNode & { children: TreeNode[] }

/** Builds the forest from the flat list the server returns (already ordered by position). */
export function buildTree(nodes: readonly DocNode[]): TreeNode[] {
  const byId = new Map<string, TreeNode>()
  for (const node of nodes) byId.set(node.id, { ...node, children: [] })
  const roots: TreeNode[] = []
  for (const node of nodes) {
    const entry = byId.get(node.id)!
    const parent = node.parentId ? byId.get(node.parentId) : undefined
    // A page whose parent is not in the list is shown at the top: the server hides what the person
    // may not read and leaves no marker.
    if (parent) parent.children.push(entry)
    else roots.push(entry)
  }
  return roots
}

export function descendantIds(nodes: readonly DocNode[], id: string): Set<string> {
  const children = new Map<string, string[]>()
  for (const node of nodes) {
    if (!node.parentId) continue
    const list = children.get(node.parentId) ?? []
    list.push(node.id)
    children.set(node.parentId, list)
  }
  const out = new Set<string>()
  const stack = [id]
  while (stack.length > 0) {
    const current = stack.pop()!
    for (const child of children.get(current) ?? []) {
      if (!out.has(child)) {
        out.add(child)
        stack.push(child)
      }
    }
  }
  return out
}

/** A page may go under any page except itself and its own subpages. */
export function canMoveUnder(nodes: readonly DocNode[], id: string, parentId: string | null): boolean {
  if (parentId === null) return true
  if (parentId === id) return false
  return !descendantIds(nodes, id).has(parentId)
}

export type FlatRow = { node: TreeNode; depth: number; hasChildren: boolean; expanded: boolean }

export function flatten(roots: readonly TreeNode[], expanded: ReadonlySet<string>): FlatRow[] {
  const rows: FlatRow[] = []
  const walk = (list: readonly TreeNode[], depth: number) => {
    for (const node of list) {
      const open = expanded.has(node.id)
      rows.push({ node, depth, hasChildren: node.children.length > 0, expanded: open })
      if (open) walk(node.children, depth + 1)
    }
  }
  walk(roots, 0)
  return rows
}

export type DocGroup = {
  key: string
  projectId: string | null
  roots: TreeNode[]
  count: number
}

function countNodes(list: readonly TreeNode[]): number {
  return list.reduce((sum, node) => sum + 1 + countNodes(node.children), 0)
}

/**
 * Roots grouped by the project the page belongs to. `projectOrder` is participating projects
 * first, then the others. A project that has pages but is missing from that list follows them.
 * Pages with no project are last. A child stays in its parent's group.
 */
export function groupByProject(roots: readonly TreeNode[], projectOrder: readonly string[]): DocGroup[] {
  const known = new Set(projectOrder)
  const buckets = new Map<string, TreeNode[]>()
  const extraIds: string[] = []
  const extra = new Map<string, TreeNode[]>()
  const none: TreeNode[] = []
  for (const root of roots) {
    const projectId = root.projectId
    if (!projectId) {
      none.push(root)
      continue
    }
    const target = known.has(projectId) ? buckets : extra
    if (!known.has(projectId) && !extra.has(projectId)) extraIds.push(projectId)
    const list = target.get(projectId) ?? []
    list.push(root)
    target.set(projectId, list)
  }
  const groups: DocGroup[] = []
  const push = (key: string, projectId: string | null, list: TreeNode[]) => {
    if (list.length === 0) return
    groups.push({ key, projectId, roots: list, count: countNodes(list) })
  }
  for (const id of projectOrder) push(id, id, buckets.get(id) ?? [])
  for (const id of extraIds) push(id, id, extra.get(id) ?? [])
  push('none', null, none)
  return groups
}

export function forestHas(roots: readonly TreeNode[], id: string): boolean {
  return findInForest(roots, id) !== null
}

export function findInForest(roots: readonly TreeNode[], id: string): TreeNode | null {
  for (const node of roots) {
    if (node.id === id) return node
    const child = findInForest(node.children, id)
    if (child) return child
  }
  return null
}

/** Pages whose title matches, plus the ancestors needed to keep them in the tree. */
export function matchingDocs(nodes: readonly DocNode[], query: string): DocNode[] {
  const needle = query.trim().toLowerCase()
  if (!needle) return [...nodes]
  const keep = new Set<string>()
  for (const node of nodes) {
    if (!node.title.toLowerCase().includes(needle)) continue
    keep.add(node.id)
    for (const ancestor of ancestorIds(nodes, node.id)) keep.add(ancestor)
  }
  return nodes.filter((node) => keep.has(node.id))
}

export function expandAll(roots: readonly TreeNode[]): Set<string> {
  const ids = new Set<string>()
  const walk = (list: readonly TreeNode[]) => {
    for (const node of list) {
      ids.add(node.id)
      walk(node.children)
    }
  }
  walk(roots)
  return ids
}

/** Every ancestor of a page, so the tree can open the path to the selected page. */
export function ancestorIds(nodes: readonly DocNode[], id: string): string[] {
  const byId = new Map(nodes.map((n) => [n.id, n]))
  const out: string[] = []
  let current = byId.get(id)?.parentId ?? null
  while (current && !out.includes(current)) {
    out.push(current)
    current = byId.get(current)?.parentId ?? null
  }
  return out
}
