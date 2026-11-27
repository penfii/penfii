import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import type { RichTextDoc } from '@penfii/contracts'
import { Button } from '../../design/ui'
import { isBlankDoc, RichTextEditor, sameRichText, type MentionCandidate } from '../../richtext'
import { withoutBlankEdges } from './commentBody'

/**
 * A comment being changed, in place of its text: the editor the comment was written with, filled with the stored body (so
 * what the comment holds is kept, not flattened), and Save and Cancel. Ctrl or ⌘ with Enter saves, Escape cancels.
 * Saving what was not changed saves nothing: the comment, its edited mark and its history stay as they were.
 */
export function CommentEditBox({
  initial,
  label,
  saveLabel,
  cancelLabel,
  onSave,
  onCancel,
  mentions,
}: {
  /** The body the box starts from; it is the "unchanged" the box compares with. */
  initial: RichTextDoc
  label: string
  saveLabel: string
  cancelLabel: string
  /** Saves the changed body. A promise that fails keeps the box and what was typed in it (the caller says what went wrong). */
  onSave: (doc: RichTextDoc) => Promise<void> | void
  onCancel: () => void
  mentions?: { search: (query: string) => Promise<MentionCandidate[]> }
}) {
  const [doc, setDoc] = useState(initial)
  const [saving, setSaving] = useState(false)
  const busy = useRef(false)
  const root = useRef<HTMLDivElement>(null)
  const blank = isBlankDoc(doc)
  // The box opens inside a scrolling list: Save and Cancel under it are brought into view with it.
  useEffect(() => {
    root.current?.scrollIntoView?.({ block: 'nearest' })
  }, [])

  async function save() {
    if (busy.current || blank) return
    const cleaned = withoutBlankEdges(doc)
    // Nothing was changed (counting an empty line added at either end as nothing): leave as it was, without a request.
    if (sameRichText(cleaned, initial) || sameRichText(doc, initial)) {
      onCancel()
      return
    }
    busy.current = true
    setSaving(true)
    try {
      await onSave(cleaned)
    } finally {
      busy.current = false
      setSaving(false)
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    // An Escape that closed the list of people (or anything else in the editor) is theirs; it does not leave the box.
    if (event.key !== 'Escape' || event.defaultPrevented || event.nativeEvent.isComposing || event.keyCode === 229) return
    event.preventDefault()
    onCancel()
  }

  return (
    <div className="comment-edit" ref={root} onKeyDown={onKeyDown}>
      <RichTextEditor value={doc} baseline={initial} onChange={setDoc} onSend={() => void save()} sendWith="mod-enter" label={label} autoFocus mentions={mentions} />
      <div className="dialog-actions">
        <Button variant="ghost" onClick={onCancel} disabled={saving}>{cancelLabel}</Button>
        <Button disabled={blank || saving} onClick={() => void save()}>{saveLabel}</Button>
      </div>
    </div>
  )
}

/**
 * After the box closes the comment's Edit button is drawn again; the keyboard goes back to it (it carries
 * `data-edit-comment="<id>"`), as it would to the control that opened a dialog.
 */
export function focusEditButton(list: HTMLElement | null, commentId: string): void {
  window.requestAnimationFrame(() => list?.querySelector<HTMLElement>(`[data-edit-comment="${commentId}"]`)?.focus())
}
