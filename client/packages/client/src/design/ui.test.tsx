import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { I18nProvider } from '../i18n/I18nProvider'
import { initials, useConfirm } from './ui'

describe('initials', () => {
  it('skips brackets and other marks and keeps up to two letters', () => {
    expect(initials('[QA] Invitee')).toBe('QI')
    expect(initials('Alice Kim')).toBe('AK')
    expect(initials('alice')).toBe('AL')
    expect(initials('김민수')).toBe('김민')
    expect(initials('  ')).toBe('?')
    expect(initials('[]')).toBe('?')
  })
})

function Probe({ onAnswer }: { onAnswer: (ok: boolean) => void }) {
  const { confirm, dialog } = useConfirm()
  return (
    <>
      <button type="button" onClick={() => void confirm({ title: 'Delete', body: 'Sure?', confirmLabel: 'Delete' }).then(onAnswer)}>ask</button>
      {dialog}
    </>
  )
}

describe('useConfirm', () => {
  beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
  afterEach(cleanup)

  async function ask(answer: string) {
    const seen: boolean[] = []
    render(<I18nProvider><Probe onAnswer={(ok) => seen.push(ok)} /></I18nProvider>)
    fireEvent.click(screen.getByText('ask'))
    expect(await screen.findByText('Sure?')).toBeTruthy()
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: answer })) })
    return seen
  }

  it('resolves true on the action and false on Cancel', async () => {
    expect(await ask('Delete')).toEqual([true])
    cleanup()
    expect(await ask('Cancel')).toEqual([false])
  })
})
