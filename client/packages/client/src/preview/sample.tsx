import type { Project, ProjectBoard, Space } from '@penfii/contracts'
import { api } from '../api'
import { useI18n } from '../i18n/I18nProvider'
import { previewSlots } from './slots'

/** Sample Projects and tasks for screenshots and demos. Loaded only by a demo build (see `loadPreview`). */

type SampleProject = {
  id: string
  name: string
  group: 'mine' | 'team'
  progress: number
  members: number
  tasks: number
  activity: string[]
  upcoming: string[]
}

const sampleProjects: SampleProject[] = [
  {
    id: 'penfii',
    name: 'penfii',
    group: 'mine',
    progress: 42,
    members: 5,
    tasks: 4,
    activity: ['보드 열을 백로그, 할 일, 진행 중, 완료로 정리했습니다.', '개요 화면에 진행률을 표시했습니다.'],
    upcoming: ['다음 주 설계 확인', '검색 화면 필터 점검'],
  },
  {
    id: 'fds',
    name: 'FDS',
    group: 'mine',
    progress: 18,
    members: 3,
    tasks: 1,
    activity: ['프로젝트 작업공간을 만들었습니다.'],
    upcoming: ['요구사항 초안'],
  },
  {
    id: 'platform',
    name: 'Platform',
    group: 'team',
    progress: 60,
    members: 8,
    tasks: 1,
    activity: ['공통 App Shell 방향을 맞췄습니다.'],
    upcoming: ['설정 화면 정리'],
  },
]

const asProject = (sample: SampleProject): Project => ({ id: sample.id, name: sample.name, teamIds: [], participating: true })
const find = (id: string) => sampleProjects.find((sample) => sample.id === id)

function SampleOverview({ project }: { project: SampleProject }) {
  const { t } = useI18n()
  return (
    <>
      <div className="metric-row">
        <article className="metric">
          <span>{t('shell.overview.progress')}</span>
          <strong>{project.progress}%</strong>
        </article>
        <article className="metric">
          <span>{t('shell.overview.tasks')}</span>
          <strong>{project.tasks}</strong>
        </article>
        <article className="metric">
          <span>{t('shell.overview.members')}</span>
          <strong>{project.members}</strong>
        </article>
      </div>
      <section className="doc-sheet">
        <h2>{t('shell.overview.activity')}</h2>
        <ul>{project.activity.map((item) => <li key={item}>{item}</li>)}</ul>
        <h2>{t('shell.overview.upcoming')}</h2>
        <ul>{project.upcoming.map((item) => <li key={item}>{item}</li>)}</ul>
      </section>
    </>
  )
}

/**
 * Makes the demo's Projects appear as ordinary ones: the Space lists them next to the real ones, and what the Project
 * header reads for them is empty but well-formed. Nothing else in the app knows they are samples.
 */
export function installSample(): void {
  // Installed already (the slot is filled): wrapping the api a second time would list every sample twice.
  if (previewSlots.overview) return
  const getSpace = api.getSpace.bind(api)
  api.getSpace = async (workspaceId: string): Promise<Space> => {
    const space = await getSpace(workspaceId)
    return { ...space, projects: [...sampleProjects.map(asProject), ...space.projects] }
  }
  const getBoard = api.getProjectBoard.bind(api)
  api.getProjectBoard = async (projectId: string): Promise<ProjectBoard> =>
    find(projectId) ? { projectId, workflowVersion: 1, canEdit: false, canManageWorkflow: false, states: [], columns: [] } : getBoard(projectId)
  const getMembers = api.getProjectMembers.bind(api)
  api.getProjectMembers = (projectId: string) => (find(projectId) ? Promise.resolve([]) : getMembers(projectId))
  previewSlots.overview = (projectId) => {
    const sample = find(projectId)
    return sample ? <SampleOverview project={sample} /> : null
  }
}
