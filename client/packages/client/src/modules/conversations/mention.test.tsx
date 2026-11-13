import type { RichTextDoc } from '@penfii/contracts'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import type { Editor } from '@tiptap/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../../i18n/I18nProvider'
import { LOCALE_STORAGE_KEY } from '../../i18n/messages'
import { QueryWrapper } from '../../test/queryWrapper'
import { Composer } from './Composer'
import { mentionRank } from './useMentionSearch'

const empty: RichTextDoc = { type: 'doc', content: [{ type: 'paragraph' }] }
const people = [
  { id: 'u-member', name: 'Mina Kim', detail: 'mina', member: true },
  { id: 'u-other', name: 'Minho Park', detail: 'minho', member: false },
]

function setup(search = vi.fn().mockResolvedValue(people), value: RichTextDoc = empty) {
  localStorage.setItem(LOCALE_STORAGE_KEY, 'en-US')
  const onSend = vi.fn().mockResolvedValue(true)
  const onChange = vi.fn()
  render(
    <QueryWrapper><I18nProvider>
      <Composer value={value} onChange={onChange} onSend={onSend} messageBodyMaxBytes={10_000} busy={false} label="message" workspaceId="w" searchMentions={search} />
    </I18nProvider></QueryWrapper>,
  )
  const textbox = screen.getByRole('textbox')
  const editor = (textbox as unknown as { editor: Editor }).editor
  return { search, onSend, onChange, textbox, editor }
}

beforeEach(() => localStorage.clear())

describe('@mention picking in the composer', () => {
  it('lists people as soon as @ is typed, then narrows as more is typed', async () => {
    const { search, editor } = setup()
    act(() => { editor.commands.insertContent('@') })
    expect(await screen.findByRole('listbox')).toBeTruthy()
    expect(search).toHaveBeenCalledWith('')
    act(() => { editor.commands.insertContent('m') })
    await waitFor(() => expect(search).toHaveBeenCalledWith('m'))
    const options = screen.getAllByRole('option')
    expect(options.map((o) => o.textContent)).toEqual(['Mina Kimmina', 'Minho Parkminho'.concat('Not a member of this channel')])
    expect(options[1].className).toContain('is-nonmember')
    expect(options[0].className).not.toContain('is-nonmember')
  })

  it('chooses with Enter instead of sending, and stores a USER mention', async () => {
    const { onSend, onChange, textbox, editor } = setup()
    act(() => { editor.commands.insertContent('hi @m') })
    await screen.findByRole('listbox')
    fireEvent.keyDown(textbox, { key: 'ArrowDown', keyCode: 40 })
    fireEvent.keyDown(textbox, { key: 'Enter', keyCode: 13 })
    expect(onSend).not.toHaveBeenCalled()
    await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull())
    const last = onChange.mock.calls.at(-1)?.[0] as RichTextDoc
    const mention = JSON.stringify(last).match(/"type":"mention","attrs":\{"occurrenceId":"[^"]+","kind":"USER","userId":"([^"]+)"\}/)
    expect(mention?.[1]).toBe('u-other')
    expect(JSON.stringify(last)).not.toContain('label')
  })

  it('leaves Enter to send when nothing matches', async () => {
    const search = vi.fn().mockResolvedValue([])
    const { onSend, textbox, editor } = setup(search, { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'ready' }] }] })
    act(() => { editor.commands.insertContent('x @zz') })
    await waitFor(() => expect(search).toHaveBeenCalledWith('zz'))
    expect(await screen.findByText('No one matches')).toBeTruthy()
    fireEvent.keyDown(textbox, { key: 'Enter', keyCode: 13, isComposing: false })
    expect(onSend).toHaveBeenCalledTimes(1)
  })

  it('does not treat an address as a mention', async () => {
    const { search, editor } = setup()
    act(() => { editor.commands.insertContent('mail me at a@b') })
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(search).not.toHaveBeenCalled()
  })
})

describe('mentionRank', () => {
  const cathy = { name: 'Cathy Park', username: 'cp', email: 'cathy.park@example.com' }
  it('fits on the name, the username and the part of the email before the @', () => {
    expect(mentionRank('cath', cathy)).toBe(0)
    expect(mentionRank('cp', cathy)).toBe(1)
    expect(mentionRank('cathy.p', { name: 'Kate', email: 'cathy.park@example.com' })).toBe(2)
    expect(mentionRank('par', cathy)).toBe(3)
    expect(mentionRank('thy', cathy)).toBe(4)
    expect(mentionRank('athy.pa', { name: 'Kate', email: 'cathy.park@example.com' })).toBe(5)
  })
  it('never fits on the domain of the email', () => {
    expect(mentionRank('example', cathy)).toBeNull()
    expect(mentionRank('com', cathy)).toBeNull()
    expect(mentionRank('c', { name: 'Bo Peep', email: 'bo@example.com' })).toBeNull()
  })
  it('accepts anyone when nothing is typed', () => {
    expect(mentionRank('', cathy)).toBe(0)
  })
})
