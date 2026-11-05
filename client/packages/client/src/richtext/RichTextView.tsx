import { createElement, type ReactNode } from 'react'
import type {
  RichTextBlock,
  RichTextDoc,
  RichTextInline,
  RichTextMark,
  RichTextText,
} from '@penfii/contracts'
import { CodeBlockView } from './CodeBlockView'
import { isSafeHttpUrl } from './url'

export type DisplayNames = Readonly<Record<string, string>>

function renderMarked(text: string, marks: readonly RichTextMark[] | undefined): ReactNode {
  return (marks ?? []).reduce<ReactNode>((child, mark) => {
    switch (mark.type) {
      case 'bold':
        return <strong>{child}</strong>
      case 'italic':
        return <em>{child}</em>
      case 'strike':
        return <s>{child}</s>
      case 'code':
        return <code>{child}</code>
      case 'link':
        if (!isSafeHttpUrl(mark.attrs.href)) return child
        return (
          <a href={mark.attrs.href} target="_blank" rel="noreferrer">
            {child}
          </a>
        )
      default:
        return child
    }
  }, text)
}

function InlineView({ node, names }: { node: RichTextInline; names?: DisplayNames }) {
  if (node.type === 'mention') {
    const label = names?.[node.attrs.userId] ?? node.attrs.userId
    return <span className="mention">@{label}</span>
  }
  return <>{renderMarked(node.text, node.marks)}</>
}

function Inlines({ nodes, names }: { nodes: readonly RichTextInline[] | undefined; names?: DisplayNames }) {
  return (
    <>
      {(nodes ?? []).map((node, index) => (
        <InlineView key={node.type === 'mention' ? node.attrs.occurrenceId : index} node={node} names={names} />
      ))}
    </>
  )
}

function textOf(nodes: readonly RichTextText[] | undefined): string {
  return (nodes ?? []).map((node) => node.text).join('')
}

function isInlineNode(node: { type?: string }): boolean {
  return node.type === 'text' || node.type === 'mention'
}

function Flow({ nodes, names }: { nodes: readonly unknown[] | undefined; names?: DisplayNames }) {
  const list = (nodes ?? []) as Array<{ type?: string }>
  if (list.length > 0 && list.every(isInlineNode)) {
    return <Inlines nodes={list as RichTextInline[]} names={names} />
  }
  return (
    <>
      {list.map((node, index) => (
        <BlockView key={index} block={node as RichTextBlock} names={names} />
      ))}
    </>
  )
}

function BlockView({ block, names }: { block: RichTextBlock; names?: DisplayNames }) {
  switch (block.type) {
    case 'paragraph':
      return (
        <p>
          <Inlines nodes={block.content} names={names} />
        </p>
      )
    case 'heading': {
      const level = Math.min(6, Math.max(1, block.attrs?.level ?? 1))
      return createElement(`h${String(level)}`, null, <Inlines nodes={block.content} names={names} />)
    }
    case 'blockquote':
      return (
        <blockquote>
          <Flow nodes={block.content} names={names} />
        </blockquote>
      )
    case 'bulletList':
      return (
        <ul>
          {(block.content ?? []).map((item, index) => (
            <li key={index}>
              <Flow nodes={item.content} names={names} />
            </li>
          ))}
        </ul>
      )
    case 'orderedList':
      return (
        <ol>
          {(block.content ?? []).map((item, index) => (
            <li key={index}>
              <Flow nodes={item.content} names={names} />
            </li>
          ))}
        </ol>
      )
    case 'codeBlock':
      return <CodeBlockView language={block.attrs?.language ?? null} text={textOf(block.content)} />
    default:
      return null
  }
}

export function RichTextView({ doc, names }: { doc: RichTextDoc; names?: DisplayNames }) {
  if (doc.type !== 'doc' || !Array.isArray(doc.content)) return null
  return (
    <div className="rich-text">
      {doc.content.map((block, index) => (
        <BlockView key={index} block={block} names={names} />
      ))}
    </div>
  )
}
