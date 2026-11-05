import {
  BlockNoteSchema,
  createHeadingBlockSpec,
  defaultBlockSpecs,
  defaultInlineContentSpecs,
} from '@blocknote/core'
import { createReactInlineContentSpec } from '@blocknote/react'
import { AUDIO_TYPES, IMAGE_TYPES, VIDEO_TYPES } from '../files/kind'
import { MentionChip, ReferenceChip } from './chips'

/**
 * The Docs editor's schema. It offers exactly what the server's DocContent v1
 * validator accepts: a block or inline node outside this list would be refused by the materializer.
 * `reference` and `mention` store ids only; their label is resolved at render time and is never part
 * of the document.
 */
import type { ReferenceKind } from '@penfii/contracts'
export type DocReferenceKind = Exclude<ReferenceKind, 'USER' | 'RECORD'>

export const referenceSpec = createReactInlineContentSpec(
  {
    type: 'reference',
    propSchema: { kind: { default: 'TICKET' as ReferenceKind }, id: { default: '' } },
    content: 'none',
  } as const,
  {
    render: (props) => <ReferenceChip kind={props.inlineContent.props.kind as ReferenceKind} id={props.inlineContent.props.id} />,
  },
)

export const mentionSpec = createReactInlineContentSpec(
  { type: 'mention', propSchema: { userId: { default: '' } }, content: 'none' } as const,
  { render: (props) => <MentionChip userId={props.inlineContent.props.userId} /> },
)

const { paragraph, bulletListItem, numberedListItem, checkListItem, quote, codeBlock, table, image, video, audio, file, divider } =
  defaultBlockSpecs

/** BlockNote's own drop and paste pick a block by `fileBlockAccept`. The defaults are `image/*` and
 * `video/*`, which would put SVG or an arbitrary video type in a media block. Only the types the
 * server will serve inline match; everything else lands on the file block. */
function accept<T extends { implementation: { meta?: { fileBlockAccept?: readonly string[] } } }>(spec: T, fileBlockAccept: readonly string[]): T {
  return { ...spec, implementation: { ...spec.implementation, meta: { ...spec.implementation.meta, fileBlockAccept } } }
}

export const docSchema = BlockNoteSchema.create({
  blockSpecs: {
    paragraph,
    heading: createHeadingBlockSpec({ levels: [1, 2, 3], allowToggleHeadings: false }),
    bulletListItem,
    numberedListItem,
    checkListItem,
    quote,
    codeBlock,
    table,
    image: accept(image, IMAGE_TYPES),
    video: accept(video, VIDEO_TYPES),
    audio: accept(audio, AUDIO_TYPES),
    file,
    divider,
  },
  inlineContentSpecs: {
    ...defaultInlineContentSpecs,
    reference: referenceSpec,
    mention: mentionSpec,
  },
})
