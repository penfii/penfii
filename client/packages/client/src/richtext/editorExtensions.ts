import { Extension, Node, mergeAttributes } from '@tiptap/core'
import Placeholder from '@tiptap/extension-placeholder'
import StarterKit from '@tiptap/starter-kit'
import { PluginKey } from '@tiptap/pm/state'
import Suggestion, { type SuggestionOptions } from '@tiptap/suggestion'
import { isSafeHttpUrl } from './url'

/** The people a mention can name: what the list shows and the id that is stored. */
export type MentionCandidate = { id: string; name: string; detail?: string; member: boolean }

export const mentionSuggestionKey = new PluginKey('mentionSuggestion')

type MentionOptions = {
  /** Where `@` opens a list. Absent: the node only keeps mentions already in the text (documents, tickets). */
  suggestion: Omit<SuggestionOptions<MentionCandidate, MentionCandidate>, 'editor'> | null
  /** The name to show for a person, when it is known. */
  nameOf: (userId: string) => string | undefined
}

/** A mention of a person. It stores the id (and an occurrence id); the name is only shown. */
export const Mention = Node.create<MentionOptions>({
  name: 'mention',
  group: 'inline',
  inline: true,
  atom: true,
  selectable: false,
  addOptions() {
    return { suggestion: null, nameOf: () => undefined }
  },
  addAttributes() {
    return {
      occurrenceId: { default: '' },
      kind: { default: 'USER' },
      userId: { default: '' },
      // The name shown while editing; it is not part of the saved text.
      label: { default: '', rendered: false },
    }
  },
  parseHTML: () => [{ tag: 'span[data-mention-id]' }],
  renderHTML: ({ HTMLAttributes }) => [
    'span',
    mergeAttributes({ class: 'mention' }, HTMLAttributes),
    `@${String(HTMLAttributes.label || '…')}`,
  ],
  addNodeView() {
    const nameOf = this.options.nameOf
    return ({ node }) => {
      const dom = document.createElement('span')
      dom.className = 'mention'
      dom.contentEditable = 'false'
      const paint = (userId: string, label: string) => {
        dom.dataset.mentionUser = userId
        dom.textContent = `@${nameOf(userId) ?? (label || '…')}`
      }
      paint(String(node.attrs.userId), String(node.attrs.label ?? ''))
      return {
        dom,
        update: (next) => {
          if (next.type !== node.type) return false
          paint(String(next.attrs.userId), String(next.attrs.label ?? ''))
          return true
        },
      }
    }
  },
  addProseMirrorPlugins() {
    const suggestion = this.options.suggestion
    return suggestion ? [Suggestion({ editor: this.editor, ...suggestion })] : []
  },
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

export function editorExtensions(placeholder: () => string, mention?: Partial<MentionOptions>) {
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
    Mention.configure(mention ?? {}),
    LinkShortcut,
    Placeholder.configure({ placeholder }),
  ]
}
