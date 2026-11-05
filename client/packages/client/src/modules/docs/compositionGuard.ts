import * as Y from 'yjs'
import {
  absolutePositionToRelativePosition,
  relativePositionToAbsolutePosition,
  ySyncPluginKey,
  type ProsemirrorBinding,
} from 'y-prosemirror'
import type { BlockNoteEditor } from '@blocknote/core'

type View = BlockNoteEditor['prosemirrorView']
type Doc = View['state']['doc']
type Observer = Parameters<ProsemirrorBinding['type']['observeDeep']>[0]

/** Blocks whose content is inline text (the others hold a table, a file or nothing). */
const TEXT_BLOCKS = new Set(['paragraph', 'heading', 'bulletListItem', 'numberedListItem', 'checkListItem', 'quote', 'codeBlock'])

/** A composition that never reports its end must not keep other people's edits away for good. */
const WATCHDOG_MS = 15_000

type Held = {
  /** The editor document when the first remote change arrived: it is what the shared document held before that change. */
  base: Doc
  /** The caret in `base`, and the same place as a position that survives remote edits. */
  head: number
  rel: unknown
}

type BindingInternals = ProsemirrorBinding & {
  _observeFunction: Observer
  _prosemirrorChanged: (doc: Doc) => void
  _forceRerender: () => void
}

/**
 * Keeps an IME composition alive while somebody else edits the same text.
 *
 * ProseMirror can only keep a composition if the text node being composed still equals what its
 * state says; a remote change inside that text node rewrites the node and the browser drops the
 * composition (the syllable is committed early, or typed twice). So while a composition is open,
 * remote changes are only noted, and local changes stay in the editor. When the composition ends
 * the shared document is rendered first and then the composed text is applied on top of it at the
 * caret's remembered place (a Yjs relative position), so the other person's text is never replaced.
 *
 * Returns the function that takes the guard off again.
 */
export function installCompositionGuard(first: View): () => void {
  const found = ySyncPluginKey.getState(first.state)?.binding as BindingInternals | undefined
  if (!found) return () => undefined
  const binding: BindingInternals = found
  // The binding is given a new view when the editor rebuilds its view (for instance when it becomes
  // editable), so the view is always asked for again.
  const view = () => binding.prosemirrorView as View

  const original = binding._observeFunction
  const originalSync = binding._prosemirrorChanged.bind(binding)
  let held: Held | null = null
  /** The caret as the shared document had it at the last moment the editor and the document agreed. */
  let anchor: { doc: Doc; head: number; rel: unknown } | null = null
  let watchdog: ReturnType<typeof setTimeout> | undefined

  const observer: Observer = (events, transaction) => {
    if (view().composing || held) {
      if (!held) hold()
      return
    }
    original(events, transaction)
  }
  binding.type.unobserveDeep(original)
  binding._observeFunction = observer
  binding.type.observeDeep(observer)
  binding._prosemirrorChanged = (doc) => {
    // The editor is behind the shared document now: writing it back would delete what is missing.
    if (held) return
    // The sync plugin calls this inside one shared-document transaction, so the text nodes of the blocks
    // the change created go out in the same update: other people never see a new block without them.
    originalSync(doc)
    ensureTextNodes(doc)
    if (view().composing) remember()
  }

  let blockCount = -1
  /**
   * A block made in the editor (Enter) is an empty element in the shared document. When two people
   * type their first characters into it at the same moment each creates a text node of their own, and
   * y-prosemirror then repairs the paragraph on both sides, which writes one person's text twice. Giving
   * the block its (empty) text node right when it is made leaves a single node for both to type into.
   * Only runs when the number of blocks changed.
   */
  function ensureTextNodes(doc: Doc) {
    let count = 0
    doc.descendants((node) => {
      if (node.type.name === 'blockContainer') count++
    })
    if (count === blockCount) return
    blockCount = count
    const missing: Y.XmlElement[] = []
    const visit = (node: Y.XmlElement | Y.XmlText | Y.XmlHook) => {
      if (!(node instanceof Y.XmlElement)) return
      if (TEXT_BLOCKS.has(node.nodeName) && node.length === 0) missing.push(node)
      node.forEach((child) => visit(child as never))
    }
    binding.type.forEach((child) => visit(child as never))
    for (const node of missing) node.insert(0, [new Y.XmlText()])
  }

  /**
   * The caret sticks to the character before it, so text that others type at the same place lands
   * after the composition instead of pushing the caret past it.
   */
  function remember() {
    const head = view().state.selection.head
    const plain = absolutePositionToRelativePosition(head, binding.type, binding.mapping)
    const place = Y.createAbsolutePositionFromRelativePosition(plain, binding.doc)
    const rel = place ? Y.createRelativePositionFromTypeIndex(place.type, place.index, -1) : plain
    anchor = { doc: view().state.doc, head, rel }
  }

  function hold() {
    // Only a caret taken while the editor matched the document can be trusted.
    const trusted = anchor && anchor.doc === view().state.doc ? anchor : null
    held = { base: view().state.doc, head: trusted?.head ?? view().state.selection.head, rel: trusted?.rel ?? null }
    clearTimeout(watchdog)
    watchdog = setTimeout(release, WATCHDOG_MS)
  }

  function release() {
    clearTimeout(watchdog)
    const current = held
    if (!current || binding.prosemirrorView == null) {
      held = null
      return
    }
    // Whatever the browser composed is read into the editor state before it is replaced.
    (view() as unknown as { domObserver: { flush(): void } }).domObserver.flush()
    const composed = view().state.doc
    const selectionHead = view().state.selection.head
    held = null
    binding._forceRerender()
    if (composed.eq(current.base)) return
    apply(current, composed, selectionHead)
  }

  function apply(current: Held, composed: Doc, selectionHead: number) {
    const start = current.base.content.findDiffStart(composed.content)
    const end = current.base.content.findDiffEnd(composed.content)
    if (start == null || !end) return
    let { a: endA, b: endB } = end
    const overlap = start - Math.min(endA, endB)
    if (overlap > 0) {
      endA += overlap
      endB += overlap
    }
    const shared = view().state.doc
    const caret = current.rel
      ? relativePositionToAbsolutePosition(binding.doc, binding.type, current.rel as never, binding.mapping)
      : null
    const shift = caret == null ? null : caret - current.head
    const from = shift == null ? null : start + shift
    const to = shift == null ? null : endA + shift
    const inserted = composed.slice(start, endB)
    const intact =
      from != null &&
      to != null &&
      from >= 0 &&
      to <= shared.content.size &&
      shared.slice(from, to).eq(current.base.slice(start, endA))
    // Otherwise (the paragraph may have been created by both people at once, which leaves the
    // remembered place without an owner) the text being replaced is looked for, nearest first.
    let at: number | null = intact ? from : null
    let replaceTo = intact ? to : null
    const replaced = current.base.textBetween(start, endA, '\n', '\ufffc')
    if (at == null && replaced) {
      const found = nearest(shared, replaced, start)
      if (found != null) {
        at = found
        replaceTo = found + replaced.length
      }
    }
    // As a last resort the composed text is only added: nothing is deleted.
    at ??= caret ?? view().state.selection.head
    replaceTo ??= at
    try {
      const tr = view().state.tr.replace(at, replaceTo, inserted)
      const within = selectionHead >= start && selectionHead <= endB
      const wanted = at + (within ? selectionHead - start : endB - start)
      // `near` is inherited from Selection by every selection class.
      const Sel = view().state.selection.constructor as unknown as { near(pos: unknown): unknown }
      tr.setSelection(Sel.near(tr.doc.resolve(Math.min(wanted, tr.doc.content.size))) as never)
      view().dispatch(tr)
    } catch (error) {
      console.warn('composition could not be placed', error)
    }
  }

  const mine = (event: Event) => event.target instanceof Node && Boolean(binding.prosemirrorView) && view().dom.contains(event.target)
  const onEnd = (event: Event) => {
    if (!mine(event)) return
    // After ProseMirror's own handler, which has already cleared `composing`.
    queueMicrotask(() => {
      if (binding.prosemirrorView && !view().composing) release()
    })
    setTimeout(() => {
      if (binding.prosemirrorView && !view().composing) release()
    }, 40)
  }
  const onStart = (event: Event) => {
    if (mine(event) && !held) remember()
  }
  document.addEventListener('compositionstart', onStart)
  document.addEventListener('compositionend', onEnd)

  return () => {
    clearTimeout(watchdog)
    document.removeEventListener('compositionstart', onStart)
    document.removeEventListener('compositionend', onEnd)
    held = null
    if (binding.prosemirrorView != null) {
      binding.type.unobserveDeep(observer)
      binding._observeFunction = original
      binding.type.observeDeep(original)
      binding._prosemirrorChanged = originalSync
    }
  }
}

/**
 * Makes a key press the editor has not yet read into a transaction, so the sync plugin writes it into
 * the shared document.
 *
 * A key press changes the DOM first; ProseMirror turns that into a transaction up to 20 ms later. A
 * remote change that is rendered in between replaces the paragraph and the key press is gone before the
 * shared document ever heard of it. This has to run BEFORE the remote update is applied to the shared
 * document (afterwards the document is ahead of the editor and writing the editor back would delete the
 * remote text), which is why the provider calls it ahead of every incoming sync message.
 */
export function readPendingInput(view: View | undefined): void {
  if (!view || view.composing) return
  const dom = (view as unknown as { domObserver?: { forceFlush?: () => void; flush?: () => void } }).domObserver
  try {
    // `forceFlush` reads what the observer has already queued, `flush` also what the browser has changed
    // in the DOM during this very task and not reported yet.
    dom?.forceFlush?.()
    dom?.flush?.()
  } catch (error) {
    console.warn('pending input could not be read', error)
  }
}

/** The position of the occurrence of `text` inside one text node that lies nearest to `near`. */
function nearest(doc: Doc, text: string, near: number): number | null {
  let best: number | null = null
  doc.descendants((node, pos) => {
    if (!node.isText || !node.text) return
    for (let at = node.text.indexOf(text); at >= 0; at = node.text.indexOf(text, at + 1)) {
      if (best == null || Math.abs(pos + at - near) < Math.abs(best - near)) best = pos + at
    }
  })
  return best
}
