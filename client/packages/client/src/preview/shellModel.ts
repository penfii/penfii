import { onlyInPreview } from './enabled'

export type TaskStatus = 'backlog' | 'todo' | 'doing' | 'done'

export type ShellTask = {
  id: string
  projectId: string
  title: string
  status: TaskStatus
  assignee: string
  comments: string[]
}

export type ShellProject = {
  id: string
  name: string
  group: 'mine' | 'team'
  progress: number
  members: number
  activity: string[]
  upcoming: string[]
}

export const shellProjects: ShellProject[] = onlyInPreview([
  {
    id: 'penfii',
    name: 'penfii',
    group: 'mine',
    progress: 42,
    members: 5,
    activity: ['보드 열을 백로그, 할 일, 진행 중, 완료로 정리했습니다.', '개요 화면에 진행률을 표시했습니다.'],
    upcoming: ['다음 주 설계 확인', '검색 화면 필터 점검'],
  },
  {
    id: 'fds',
    name: 'FDS',
    group: 'mine',
    progress: 18,
    members: 3,
    activity: ['프로젝트 작업공간을 만들었습니다.'],
    upcoming: ['요구사항 초안'],
  },
  {
    id: 'platform',
    name: 'Platform',
    group: 'team',
    progress: 60,
    members: 8,
    activity: ['공통 App Shell 방향을 맞췄습니다.'],
    upcoming: ['설정 화면 정리'],
  },
])

const seedTasks: ShellTask[] = onlyInPreview([
  { id: 'shell', projectId: 'penfii', title: 'App Shell 정리', status: 'doing', assignee: 'Alice', comments: ['Global Nav와 Context Sidebar를 분리합니다.'] },
  { id: 'board', projectId: 'penfii', title: '보드를 Task 보기로 두기', status: 'todo', assignee: 'Bob', comments: [] },
  { id: 'search', projectId: 'penfii', title: '검색 필터 자리', status: 'backlog', assignee: 'Charlie', comments: ['사람, 메시지, 조직을 같은 검색에 둡니다.'] },
  { id: 'detail', projectId: 'penfii', title: '상세 패널', status: 'done', assignee: 'David', comments: ['선택한 작업만 오른쪽에서 엽니다.'] },
  { id: 'fds-scope', projectId: 'fds', title: '범위 적기', status: 'todo', assignee: 'Alice', comments: [] },
  { id: 'platform-nav', projectId: 'platform', title: '공통 내비게이션', status: 'doing', assignee: 'Bob', comments: [] },
])

export function tasksFor(projectId: string): ShellTask[] {
  return seedTasks.filter((task) => task.projectId === projectId)
}

export function taskById(id: string): ShellTask | undefined {
  return seedTasks.find((task) => task.id === id)
}

export function shellProject(id: string | undefined): ShellProject | undefined {
  return shellProjects.find((project) => project.id === id)
}
