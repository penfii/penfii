import type { ProjectBoard, ProjectMember, ProjectPlanning, TicketLabel, TicketType } from '@penfii/contracts'

/** What the detail panel needs from the Ticket's Project to show and change its fields. */
export type ProjectContext = {
  board: ProjectBoard
  labels: TicketLabel[]
  members: ProjectMember[]
  types: TicketType[]
  /** Sprints and Milestones; undefined while loading. */
  planning: ProjectPlanning | undefined
}
