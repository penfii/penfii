import { Editor } from '@tiptap/core'
import { describe, expect, it } from 'vitest'
import { editorToRichText } from './adapter'
import { toggleCodeBlock, toggleInlineCode } from './codeCommands'
import { editorExtensions } from './editorExtensions'

function editorWith(text: string): Editor {
  return new Editor({
    extensions: editorExtensions(() => ''),
    content: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }] }] },
  })
}

describe('toolbar code commands', () => {
  it('toggles inline code on the selection', () => {
    const editor = editorWith('userId')
    editor.commands.setTextSelection({ from: 1, to: 7 })
    toggleInlineCode(editor)
    const stored = editorToRichText(editor.getJSON(), null)
    expect(stored.content?.[0]).toMatchObject({
      type: 'paragraph',
      content: [{ type: 'text', text: 'userId', marks: [{ type: 'code' }] }],
    })
    editor.commands.setTextSelection({ from: 1, to: 7 })
    toggleInlineCode(editor)
    expect(editorToRichText(editor.getJSON(), null).content?.[0]).toMatchObject({
      type: 'paragraph',
      content: [{ type: 'text', text: 'userId' }],
    })
    editor.destroy()
  })

  it('joins selected paragraphs into one code block and restores them', () => {
    const editor = new Editor({
      extensions: editorExtensions(() => ''),
      content: {
        type: 'doc',
        content: [
          { type: 'paragraph', content: [{ type: 'text', text: 'const user = await getUser();' }] },
          { type: 'paragraph', content: [{ type: 'text', text: 'console.log(user);' }] },
        ],
      },
    })
    editor.commands.setTextSelection({ from: 1, to: editor.state.doc.content.size - 1 })
    toggleCodeBlock(editor)
    expect(editorToRichText(editor.getJSON(), null)).toMatchObject({
      content: [{
        type: 'codeBlock',
        attrs: { language: null },
        content: [{ type: 'text', text: 'const user = await getUser();\nconsole.log(user);' }],
      }],
    })
    toggleCodeBlock(editor)
    expect(editorToRichText(editor.getJSON(), null).content?.map((block) => block.type)).toEqual(['paragraph', 'paragraph'])
    editor.destroy()
  })

  it('turns only the selected lines into one code block', () => {
    const editor = new Editor({
      extensions: editorExtensions(() => ''),
      content: {
        type: 'doc',
        content: [
          { type: 'paragraph', content: [{ type: 'text', text: '안녕하세요.' }] },
          { type: 'paragraph', content: [{ type: 'text', text: 'const user = getUser();' }] },
          { type: 'paragraph', content: [{ type: 'text', text: 'console.log(user);' }] },
          { type: 'paragraph', content: [{ type: 'text', text: '다음 내용입니다.' }] },
        ],
      },
    })
    let from = 0
    let to = 0
    editor.state.doc.descendants((node, pos) => {
      if (!node.isText || !node.text) return
      if (node.text.startsWith('const user')) from = pos + 4
      if (node.text.startsWith('console.log')) to = pos + 6
    })
    editor.commands.setTextSelection({ from, to })
    toggleCodeBlock(editor)
    const stored = editorToRichText(editor.getJSON(), null)
    expect(stored.content?.map((block) => block.type)).toEqual(['paragraph', 'codeBlock', 'paragraph'])
    expect(stored.content?.[1]).toMatchObject({
      type: 'codeBlock',
      attrs: { language: null },
      content: [{ type: 'text', text: 'const user = getUser();\nconsole.log(user);' }],
    })
    toggleCodeBlock(editor)
    expect(editorToRichText(editor.getJSON(), null).content?.map((block) => block.type)).toEqual([
      'paragraph',
      'paragraph',
      'paragraph',
      'paragraph',
    ])
    editor.destroy()
  })

  it('splits one line out of a paragraph and leaves the surrounding lines', () => {
    const editor = new Editor({
      extensions: editorExtensions(() => ''),
      content: {
        type: 'doc',
        content: [{
          type: 'paragraph',
          content: [
            { type: 'text', text: '첫 번째 줄', marks: [{ type: 'bold' }] },
            { type: 'hardBreak' },
            { type: 'text', text: '두 번째 줄' },
            { type: 'hardBreak' },
            { type: 'text', text: '세 번째 줄', marks: [{ type: 'italic' }] },
          ],
        }],
      },
    })
    let from = 0
    let to = 0
    editor.state.doc.descendants((node, pos) => {
      if (node.isText && node.text === '두 번째 줄') {
        from = pos + 1
        to = pos + 4
      }
    })
    editor.commands.setTextSelection({ from, to })
    toggleCodeBlock(editor)
    const stored = editorToRichText(editor.getJSON(), null)
    expect(stored.content?.map((block) => block.type)).toEqual(['paragraph', 'codeBlock', 'paragraph'])
    expect(stored.content?.[0]).toMatchObject({
      type: 'paragraph',
      content: [{ type: 'text', text: '첫 번째 줄', marks: [{ type: 'bold' }] }],
    })
    expect(stored.content?.[1]).toMatchObject({
      type: 'codeBlock',
      attrs: { language: null },
      content: [{ type: 'text', text: '두 번째 줄' }],
    })
    expect(stored.content?.[2]).toMatchObject({
      type: 'paragraph',
      content: [{ type: 'text', text: '세 번째 줄', marks: [{ type: 'italic' }] }],
    })
    toggleCodeBlock(editor)
    expect(editorToRichText(editor.getJSON(), null).content?.map((block) => block.type)).toEqual([
      'paragraph',
      'paragraph',
      'paragraph',
    ])
    editor.destroy()
  })

  it('keeps paragraphs outside a two-line selection', () => {
    const editor = new Editor({
      extensions: editorExtensions(() => ''),
      content: {
        type: 'doc',
        content: [
          { type: 'paragraph', content: [{ type: 'text', text: '첫 번째 줄' }] },
          { type: 'paragraph', content: [{ type: 'text', text: '두 번째 줄' }] },
          { type: 'paragraph', content: [{ type: 'text', text: '세 번째 줄' }] },
          { type: 'paragraph', content: [{ type: 'text', text: '네 번째 줄' }] },
        ],
      },
    })
    let from = 0
    let to = 0
    editor.state.doc.descendants((node, pos) => {
      if (!node.isText || !node.text) return
      if (node.text === '두 번째 줄') from = pos + 1
      if (node.text === '세 번째 줄') to = pos + node.text.length
    })
    editor.commands.setTextSelection({ from, to })
    toggleCodeBlock(editor)
    const stored = editorToRichText(editor.getJSON(), null)
    expect(stored.content?.map((block) => block.type)).toEqual(['paragraph', 'codeBlock', 'paragraph'])
    expect(stored.content?.[1]).toMatchObject({
      content: [{ type: 'text', text: '두 번째 줄\n세 번째 줄' }],
    })
    expect(stored.content?.[0]).toMatchObject({ content: [{ type: 'text', text: '첫 번째 줄' }] })
    expect(stored.content?.[2]).toMatchObject({ content: [{ type: 'text', text: '네 번째 줄' }] })
    editor.destroy()
  })

  it('keeps a code block language while the block stays a code block', () => {
    const editor = new Editor({
      extensions: editorExtensions(() => ''),
      content: {
        type: 'doc',
        content: [{
          type: 'codeBlock',
          attrs: { language: 'typescript' },
          content: [{ type: 'text', text: 'const hello = "world";' }],
        }],
      },
    })
    expect(editorToRichText(editor.getJSON(), null).content?.[0]).toMatchObject({
      type: 'codeBlock',
      attrs: { language: 'typescript' },
    })
    editor.destroy()
  })
})
