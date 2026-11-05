import type { BoardColumn, ProjectBoard, Ticket, WorkflowState } from '@penfii/contracts'

export const fixtureStates: WorkflowState[] = [
  { id: 's-todo', projectId: 'p', name: 'Todo', category: 'NOT_STARTED', position: 0, isInitial: true, wipLimit: null },
  { id: 's-doing', projectId: 'p', name: 'In Progress', category: 'IN_PROGRESS', position: 1, isInitial: false, wipLimit: null },
  { id: 's-done', projectId: 'p', name: 'Done', category: 'DONE', position: 2, isInitial: false, wipLimit: null },
]

export function fixtureTicket(partial: Partial<Ticket> = {}): Ticket {
  return {
    id: 't1', workspaceId: 'w', projectId: 'p', channelId: null, key: 'PFI-001', title: 'Fix login', description: {},
    hierarchyKind: 'TASK', parentId: null, statusId: 's-todo', assigneeId: null, assigneeName: null, labels: [], priority: null,
    dueOn: null, ticketType: null, parent: null, childSummary: { total: 0, done: 0, canceled: 0 },
    dependencies: { openBlockers: 0, blockedBy: 0, blocks: 0 }, sprint: null, milestones: [], revisionNo: 1,
    createdAt: '2026-10-19T00:00:00Z', updatedAt: '2026-10-19T00:00:00Z', ...partial,
  }
}

export function fixtureColumn(state: WorkflowState, tickets: Ticket[]): BoardColumn {
  const own = tickets.filter((item) => item.statusId === state.id)
  return {
    id: state.id, name: state.name, category: state.category, position: state.position, stateId: state.id, stateIds: [state.id],
    ticketCount: own.length, wipLimit: state.wipLimit, tickets: own,
  }
}

export function fixtureBoard(tickets: Ticket[], canEdit = true, states: WorkflowState[] = fixtureStates): ProjectBoard {
  return {
    projectId: 'p', workflowVersion: 1, canEdit, canManageWorkflow: false, states,
    columns: states.map((state) => fixtureColumn(state, tickets)),
  }
}
