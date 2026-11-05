import { describe, expect, it } from 'vitest'
import { visibilityHint, visibilityLabel } from './visibilityLabel'

const t = (key: string) => key as never

describe('channel visibility wording', () => {
  it('names a Project channel for the people who can read it, not as public', () => {
    expect(visibilityLabel(t, 'PUBLIC', true)).toBe('channel.projectPublic')
    expect(visibilityLabel(t, 'PUBLIC', false)).toBe('channel.public')
    expect(visibilityLabel(t, 'PRIVATE', true)).toBe('channel.private')
    expect(visibilityHint(t, true)).toBe('channel.hint.project')
    expect(visibilityHint(t, false)).toBe('channel.hint.workspace')
  })
})
