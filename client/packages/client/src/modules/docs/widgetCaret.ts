import type { BlockNoteEditor } from '@blocknote/core'

type View = BlockNoteEditor['prosemirrorView']

/**
 * Keeps the caret out of other people's cursor flags.
 *
 * A remote cursor is drawn as a `contenteditable=false` widget inside the paragraph. In an empty
 * paragraph it is all there is to click on, and when it is redrawn (somebody else moved or typed) the
 * browser can leave the caret inside it. ProseMirror still believes the selection is fine, but the
 * browser does not insert text into a non-editable element: the key press fires `keydown` and nothing
 * else, and the character never reaches the editor. The caret is moved to just after the widget, which
 * is the same document position, before the key is handled.
 */
export function installWidgetCaret(view: () => View | undefined): () => void {
  const inWidget = (): Element | null => {
    const sel = document.getSelection()
    const root = view()?.dom
    if (!root || !sel || !sel.isCollapsed || !sel.anchorNode || !root.contains(sel.anchorNode)) return null
    const start = sel.anchorNode instanceof Element ? sel.anchorNode : sel.anchorNode.parentElement
    const widget = start?.closest('.ProseMirror-widget[contenteditable="false"]') ?? null
    return widget && root.contains(widget) && widget.parentNode ? widget : null
  }
  const leave = () => {
    const widget = inWidget()
    const sel = document.getSelection()
    if (!widget || !sel || !widget.parentNode) return
    const at = Array.prototype.indexOf.call(widget.parentNode.childNodes, widget) + 1
    sel.collapse(widget.parentNode, at)
  }
  // A composition in progress is not interrupted: its own events would be disturbed by a moving caret.
  const guarded = () => {
    const current = view()
    if (!current || current.composing || !current.hasFocus()) return
    leave()
  }
  const onKey = (event: Event) => {
    const root = view()?.dom
    if (root && event.target instanceof Node && root.contains(event.target)) guarded()
  }
  document.addEventListener('keydown', onKey, true)
  document.addEventListener('selectionchange', guarded)
  return () => {
    document.removeEventListener('keydown', onKey, true)
    document.removeEventListener('selectionchange', guarded)
  }
}
