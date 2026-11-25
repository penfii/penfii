import { fireEvent, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { renderWithProviders } from '../test/renderWithProviders'
import { ActionMenu } from './ActionMenu'

describe('ActionMenu', () => {
  it('opens a menu that is placed from the button, not inside a clipping table wrapper', () => {
    const onSelect = vi.fn()
    renderWithProviders(<ActionMenu label="Actions for Mina" items={[{ key: 'a', label: 'Make Admin', onSelect }, { key: 'b', label: 'Remove', onSelect, danger: true }]} />)
    fireEvent.click(screen.getByRole('button', { name: 'Actions for Mina' }))
    const menu = screen.getByRole('menu')
    expect(menu.style.position).toBe('fixed')
    expect(screen.getAllByRole('menuitem').map((item) => item.textContent)).toEqual(['Make Admin', 'Remove'])
    fireEvent.click(screen.getByRole('menuitem', { name: 'Make Admin' }))
    expect(onSelect).toHaveBeenCalledTimes(1)
    expect(screen.queryByRole('menu')).toBeNull()
  })

  it('closes on Escape and keeps the page scroll from closing it', () => {
    renderWithProviders(<ActionMenu label="Actions" items={[{ key: 'a', label: 'One', onSelect: () => {} }]} />)
    fireEvent.click(screen.getByRole('button', { name: 'Actions' }))
    fireEvent.scroll(window)
    expect(screen.getByRole('menu')).toBeTruthy()
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()
  })
})
