// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import * as Y from 'yjs'
import { EditorState } from '@tiptap/pm/state'
import { EditorView } from '@tiptap/pm/view'
import { Schema } from '@tiptap/pm/model'
import { ySyncPlugin } from 'y-prosemirror'
import { installCompositionGuard } from './compositionGuard'

const schema = new Schema({
  nodes: {
    doc: { content: 'paragraph+' },
    paragraph: { content: 'text*', toDOM: () => ['p', 0], parseDOM: [{ tag: 'p' }] },
    text: {},
  },
})

type Composing = { input: { composing: boolean } }

/** A view bound to a Yjs fragment, with a second Yjs document standing for another person. */
function setup(initial: string) {
  const local = new Y.Doc()
  const remote = new Y.Doc()
  const view = new EditorView(document.createElement('div'), {
    state: EditorState.create({ schema, plugins: [ySyncPlugin(local.getXmlFragment('f'))] }),
  })
  document.body.append(view.dom)
  // The two documents exchange updates the way the server would.
  const send = (from: Y.Doc, to: Y.Doc) => from.on('update', (update: Uint8Array, origin: unknown) => origin !== 'peer' && Y.applyUpdate(to, update, 'peer'))
  send(local, remote)
  send(remote, local)
  view.dispatch(view.state.tr.insertText(initial, 1))
  const dispose = installCompositionGuard(view as never)
  const text = () => view.state.doc.textContent
  const remoteText = () => (remote.getXmlFragment('f').get(0) as Y.XmlElement).toString().replace(/<[^>]+>/g, '')
  return { view, remote, text, remoteText, dispose }
}

/** Another person typing into the shared paragraph at `index`. */
function remoteInsert(remote: Y.Doc, index: number, value: string) {
  const paragraph = remote.getXmlFragment('f').get(0) as Y.XmlElement
  ;(paragraph.get(0) as Y.XmlText).insert(index, value)
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 60))

describe('installCompositionGuard', () => {
  const disposers: Array<() => void> = []
  afterEach(() => {
    for (const dispose of disposers.splice(0)) dispose()
    document.body.replaceChildren()
  })

  it('keeps the editor text still while composing and merges the result afterwards', async () => {
    const s = setup('가나다')
    disposers.push(s.dispose)
    const composing = s.view as unknown as Composing
    composing.input.composing = true
    s.view.dispatch(s.view.state.tr.insertText('ㅎ', s.view.state.selection.head))
    s.view.dom.dispatchEvent(new Event('compositionstart', { bubbles: true }))

    // someone types at the same place while the syllable is open
    remoteInsert(s.remote, 4, 'b0')
    expect(s.text()).toBe('가나다ㅎ')

    // the syllable becomes 한
    const from = s.view.state.selection.head - 1
    s.view.dispatch(s.view.state.tr.insertText('한', from, from + 1))
    composing.input.composing = false
    s.view.dom.dispatchEvent(new Event('compositionend', { bubbles: true }))
    await settle()

    expect(s.text()).toBe('가나다한b0')
    expect(s.remoteText()).toBe('가나다한b0')
  })

  it('does not write the stale editor back over the other person\'s text', async () => {
    const s = setup('가나다')
    disposers.push(s.dispose)
    const composing = s.view as unknown as Composing
    composing.input.composing = true
    s.view.dom.dispatchEvent(new Event('compositionstart', { bubbles: true }))
    remoteInsert(s.remote, 0, 'X')
    remoteInsert(s.remote, 4, 'Y')
    s.view.dispatch(s.view.state.tr.insertText('ㅎ', s.view.state.selection.head))
    expect(s.remoteText()).toBe('X가나다Y')
    composing.input.composing = false
    s.view.dom.dispatchEvent(new Event('compositionend', { bubbles: true }))
    await settle()
    expect(s.remoteText()).toContain('X')
    expect(s.remoteText()).toContain('Y')
    expect(s.text()).toBe(s.remoteText())
  })

  it('applies remote changes at once when nobody is composing', () => {
    const s = setup('가나다')
    disposers.push(s.dispose)
    remoteInsert(s.remote, 3, '!')
    expect(s.text()).toBe('가나다!')
  })

  it('stops holding back after being removed', () => {
    const s = setup('가나다')
    s.dispose()
    ;(s.view as unknown as Composing).input.composing = true
    remoteInsert(s.remote, 3, '!')
    expect(s.text()).toBe('가나다!')
  })
})
