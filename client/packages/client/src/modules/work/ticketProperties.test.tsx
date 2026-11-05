import { cleanup, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Ticket } from '@penfii/contracts'

vi.mock('../../api', () => ({ api: {} }))

import { TicketProperties } from './TicketProperties'
import type { ProjectContext } from './projectContext'
import { renderWithProviders } from '../../test/renderWithProviders'

const ctx = { board: { states: [], canEdit: true, canManageWorkflow: false }, members: [], labels: [], types: [], sprints: [], milestones: [] } as unknown as ProjectContext
const ticket = { id: 't', projectId: 'p', statusId: 's', labels: [], priority: 'NONE', assigneeId: null, dueOn: null } as unknown as Ticket

function view(canEdit: boolean) {
  renderWithProviders(<TicketProperties ticket={ticket} ctx={ctx} canEdit={canEdit} busy={false} onSave={() => undefined} onStatus={() => undefined} />)
}

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(cleanup)

describe('Label creation is Collaborate', () => {
  it('is offered to an editor who is not the Project admin', () => {
    view(true)
    expect(screen.getByLabelText('New label name')).toBeTruthy()
  })

  it('is not offered to a reader', () => {
    view(false)
    expect(screen.queryByLabelText('New label name')).toBeNull()
  })
})
