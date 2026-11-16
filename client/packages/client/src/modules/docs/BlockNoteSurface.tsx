import '@blocknote/mantine/style.css'
import { MantineProvider } from '@mantine/core'
import { en as enDictionary, ko as koDictionary } from '@blocknote/core/locales'
import type { BlockNoteEditor } from '@blocknote/core'
import { FormattingToolbar, FormattingToolbarController } from '@blocknote/react'
import { BlockNoteView } from '@blocknote/mantine'
import type { ReactNode } from 'react'
import { FormatButtons, PhoneFormatBar } from './FormatBar'

/** Shared Docs BlockNote surface: schema/editor creation remains with the caller; rendering and toolbar stay consistent. */
export function BlockNoteSurface({ editor, editable, children }: { editor: BlockNoteEditor<any, any, any>; editable: boolean; children?: ReactNode }) {
  return (
    <MantineProvider withGlobalClasses={false} withCssVariables={false}>
      <BlockNoteView editor={editor} theme="light" slashMenu={false} formattingToolbar={false} editable={editable}>
        <FormattingToolbarController formattingToolbar={() => <FormattingToolbar><FormatButtons /></FormattingToolbar>} />
        {editable ? <PhoneFormatBar /> : null}
        {children}
      </BlockNoteView>
    </MantineProvider>
  )
}

export function blockNoteDictionary(locale: string) {
  return locale === 'ko-KR' ? koDictionary : enDictionary
}
