import type { MessageKey } from '../i18n/messages'
import { onlyInPreview } from './enabled'

export const previewDocs: Array<{ id: string; titleKey: MessageKey; bodyKey: MessageKey }> = onlyInPreview([
  { id: 'principles', titleKey: 'preview.doc.principles', bodyKey: 'preview.doc.principlesBody' },
  { id: 'handbook', titleKey: 'preview.doc.handbook', bodyKey: 'preview.doc.handbookBody' },
  { id: 'design', titleKey: 'preview.doc.design', bodyKey: 'preview.doc.designBody' },
])
