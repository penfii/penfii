import { Extension, Node, mergeAttributes } from '@tiptap/core'
import Placeholder from '@tiptap/extension-placeholder'
import StarterKit from '@tiptap/starter-kit'
import { isSafeHttpUrl } from './url'

/** Keeps an existing mention in the document. Mention picking is a later step. */
export const Mention = Node.create({
  name: 'mention',
  group: 'inline',
  inline: true,
  atom: true,
  selectable: false,
  addAttributes() {
    return {
      occurrenceId: { default: '' },
      kind: { default: 'USER' },
      userId: { default: '' },
    }
  },
  parseHTML: () => [{ tag: 'span[data-mention-id]' }],
  renderHTML: ({ HTMLAttributes }) => [
    'span',
    mergeAttributes({ class: 'mention' }, HTMLAttributes),
    `@${String(HTMLAttributes.userId ?? '')}`,
  ],
})

export const LinkShortcut = Extension.create<Record<string, never>, { open: () => void }>({
  name: 'linkShortcut',
  addStorage() {
    return { open: () => undefined }
  },
  addKeyboardShortcuts() {
    return {
      'Mod-k': () => {
        this.storage.open()
        return true
      },
    }
  },
})

export function editorExtensions(placeholder: () => string) {
  return [
    StarterKit.configure({
      horizontalRule: false,
      underline: false,
      trailingNode: false,
      heading: { levels: [1, 2, 3, 4, 5, 6] },
      link: {
        openOnClick: false,
        autolink: false,
        linkOnPaste: true,
        markdownLinks: true,
        protocols: ['http', 'https'],
        validate: isSafeHttpUrl,
        isAllowedUri: (url) => isSafeHttpUrl(url),
      },
    }),
    Mention,
    LinkShortcut,
    Placeholder.configure({ placeholder }),
  ]
}
