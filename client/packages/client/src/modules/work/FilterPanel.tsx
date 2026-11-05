import { useState } from 'react'
import type { Channel, EstimateFilter, HierarchyKind, ProjectMember, ProjectPlanning, ScopeProject, TicketLabel, TicketType, ViewFilter, WorkflowState } from '@penfii/contracts'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { isEstimateText, normalizeEstimateText } from './planValues'
import { PRIORITIES } from './ticketModel'
import { CATEGORIES, emptyFilter, filterCount } from './viewModel'

const KINDS: HierarchyKind[] = ['EPIC', 'TASK', 'SUBTASK']

function toggle(list: string[], value: string): string[] {
  return list.includes(value) ? list.filter((item) => item !== value) : [...list, value]
}

function Choice({ checked, onChange, children }: { checked: boolean; onChange: () => void; children: string }) {
  return (
    <label className="filter-choice">
      <input type="checkbox" checked={checked} onChange={onChange} />
      <span>{children}</span>
    </label>
  )
}

/** Estimate: any, has one, has none, or at least / at most a value (one Project only: units differ). */
function EstimateCriterion({ value, wide, onChange }: { value: EstimateFilter | null | undefined; wide: boolean; onChange: (value: EstimateFilter | null) => void }) {
  const { t } = useI18n()
  // What is being typed stays while it is not a number yet; the criterion changes once it is.
  const [text, setText] = useState(value?.value ?? '')
  const op = value?.op ?? ''
  const compares = op === 'GTE' || op === 'LTE'
  return (
    <fieldset className="filter-group estimate-filter">
      <legend>{t('view.filter.estimate')}</legend>
      <select
        aria-label={t('view.filter.estimate')}
        value={op}
        onChange={(event) => {
          const next = event.target.value
          if (next === '') onChange(null)
          else if (next === 'SET' || next === 'UNSET') onChange({ op: next })
          else if (next === 'GTE' || next === 'LTE') onChange(isEstimateText(text) ? { op: next, value: normalizeEstimateText(text) } : { op: next, value: '' })
        }}
      >
        <option value="">{t('view.filter.estimate.any')}</option>
        <option value="SET">{t('view.filter.estimate.SET')}</option>
        <option value="UNSET">{t('view.filter.estimate.UNSET')}</option>
        {wide ? null : <option value="GTE">{t('view.filter.estimate.GTE')}</option>}
        {wide ? null : <option value="LTE">{t('view.filter.estimate.LTE')}</option>}
      </select>
      {compares ? (
        <input
          type="text"
          inputMode="decimal"
          aria-label={t('view.filter.estimate.value')}
          value={text}
          onChange={(event) => {
            setText(event.target.value)
            if (isEstimateText(event.target.value) && value) onChange({ op: value.op, value: normalizeEstimateText(event.target.value) })
          }}
        />
      ) : null}
      {wide ? <small className="hint">{t('view.filter.estimate.projectOnly')}</small> : null}
    </fieldset>
  )
}

function Group({ title, empty, children }: { title: string; empty?: boolean; children: React.ReactNode }) {
  const { t } = useI18n()
  return (
    <fieldset className="filter-group">
      <legend>{title}</legend>
      {empty ? <small className="hint">{t('view.filter.empty')}</small> : children}
    </fieldset>
  )
}

/**
 * The conditions of a View. Values inside one group are alternatives; groups all have to match.
 * Labels, types, Sprints, Milestones and channels belong to one Project, so a Team or Workspace
 * scope (`projects` given) filters by Project and State category instead.
 */
export function FilterPanel({
  filter,
  labels,
  members,
  types,
  planning,
  channels = [],
  projects,
  onChange,
}: {
  filter: ViewFilter
  labels: TicketLabel[]
  members: ProjectMember[]
  types: TicketType[]
  planning: ProjectPlanning | undefined
  channels?: Channel[]
  projects?: ScopeProject[]
  onChange: (filter: ViewFilter) => void
}) {
  const { t } = useI18n()
  const sprints = planning?.sprints.filter((sprint) => sprint.state === 'PLANNED' || sprint.state === 'ACTIVE') ?? []
  const milestones = planning?.milestones ?? []
  const set = (patch: Partial<ViewFilter>) => onChange({ ...filter, ...patch })
  return (
    <div className="filter-panel">
      <div className="filter-panel-head">
        <strong>{t('view.filter')}</strong>
        <Button type="button" variant="ghost" disabled={filterCount(filter) === 0} onClick={() => onChange(emptyFilter())}>{t('view.filter.clear')}</Button>
      </div>
      <Group title={t('view.filter.category')}>
        {CATEGORIES.map((category) => (
          <Choice key={category} checked={filter.categories.includes(category)} onChange={() => set({ categories: toggle(filter.categories, category) as WorkflowState['category'][] })}>{t(`work.category.${category}`)}</Choice>
        ))}
      </Group>
      {projects ? (
        <Group title={t('view.filter.project')} empty={projects.length === 0}>
          {projects.map((project) => (
            <Choice key={project.id} checked={filter.projectIds.includes(project.id)} onChange={() => set({ projectIds: toggle(filter.projectIds, project.id) })}>{project.name}</Choice>
          ))}
        </Group>
      ) : (
        <>
          <Group title={t('view.filter.channel')} empty={channels.length === 0}>
            {channels.map((channel) => (
              <Choice key={channel.id} checked={filter.channelIds.includes(channel.id)} onChange={() => set({ channelIds: toggle(filter.channelIds, channel.id) })}>{`# ${channel.name}`}</Choice>
            ))}
          </Group>
          <Group title={t('view.filter.labels')} empty={labels.length === 0}>
            {labels.map((label) => (
              <Choice key={label.id} checked={filter.labelIds.includes(label.id)} onChange={() => set({ labelIds: toggle(filter.labelIds, label.id) })}>{label.name}</Choice>
            ))}
          </Group>
        </>
      )}
      <Group title={t('view.filter.assignee')}>
        <Choice checked={filter.unassigned} onChange={() => set({ unassigned: !filter.unassigned })}>{t('view.filter.unassigned')}</Choice>
        {members.map((member) => (
          <Choice key={member.userId} checked={filter.assigneeIds.includes(member.userId)} onChange={() => set({ assigneeIds: toggle(filter.assigneeIds, member.userId) })}>{member.displayName}</Choice>
        ))}
      </Group>
      <Group title={t('view.filter.priority')}>
        {PRIORITIES.map((priority) => (
          <Choice key={priority} checked={filter.priorities.includes(priority)} onChange={() => set({ priorities: toggle(filter.priorities, priority) })}>{t(`work.priority.${priority}`)}</Choice>
        ))}
      </Group>
      {projects ? null : (
        <Group title={t('view.filter.type')} empty={types.length === 0}>
          {types.map((type) => (
            <Choice key={type.id} checked={filter.typeIds.includes(type.id)} onChange={() => set({ typeIds: toggle(filter.typeIds, type.id) })}>{type.name}</Choice>
          ))}
        </Group>
      )}
      <Group title={t('view.filter.kind')}>
        {KINDS.map((kind) => (
          <Choice key={kind} checked={filter.kinds.includes(kind)} onChange={() => set({ kinds: toggle(filter.kinds, kind) as HierarchyKind[] })}>{t(`work.kind.${kind}`)}</Choice>
        ))}
      </Group>
      {projects ? null : (
        <>
          <Group title={t('view.filter.sprint')}>
            <Choice checked={filter.noSprint} onChange={() => set({ noSprint: !filter.noSprint })}>{t('view.filter.noSprint')}</Choice>
            {sprints.map((sprint) => (
              <Choice key={sprint.id} checked={filter.sprintIds.includes(sprint.id)} onChange={() => set({ sprintIds: toggle(filter.sprintIds, sprint.id) })}>{sprint.name}</Choice>
            ))}
          </Group>
          <Group title={t('view.filter.milestone')} empty={milestones.length === 0}>
            {milestones.map((milestone) => (
              <Choice key={milestone.id} checked={filter.milestoneIds.includes(milestone.id)} onChange={() => set({ milestoneIds: toggle(filter.milestoneIds, milestone.id) })}>{milestone.name}</Choice>
            ))}
          </Group>
        </>
      )}
      <EstimateCriterion value={filter.estimate} wide={projects !== undefined} onChange={(estimate) => set({ estimate })} />
      <label className="filter-select">
        <span>{t('view.filter.blocked')}</span>
        <select
          value={filter.blocked === null ? '' : filter.blocked ? 'yes' : 'no'}
          onChange={(event) => set({ blocked: event.target.value === '' ? null : event.target.value === 'yes' })}
        >
          <option value="">{t('view.filter.blocked.any')}</option>
          <option value="yes">{t('view.filter.blocked.yes')}</option>
          <option value="no">{t('view.filter.blocked.no')}</option>
        </select>
      </label>
    </div>
  )
}
