import type { Department, Team, WorkspacePerson } from '@penfii/contracts'

/** One staged change of the department tree: a Team moves to a department, or a department to a new parent. */
export type OrgMove = {
  kind: 'team' | 'department'
  id: string
  name: string
  /** Where the item sits now, and where it would sit after the change; `null` is "no department" / the top. */
  from: string | null
  to: string | null
}

/** A department and everything below it. */
export function subtreeIds(departments: Array<Pick<Department, 'id' | 'parentId'>>, rootId: string): Set<string> {
  const found = new Set<string>([rootId])
  let grew = true
  while (grew) {
    grew = false
    for (const department of departments) {
      if (department.parentId && found.has(department.parentId) && !found.has(department.id)) {
        found.add(department.id)
        grew = true
      }
    }
  }
  return found
}

/** Adds a change to the staged list. Moving the same item again replaces the earlier change, and moving it back drops it. */
export function stageMove(moves: OrgMove[], next: OrgMove): OrgMove[] {
  const earlier = moves.find((move) => move.kind === next.kind && move.id === next.id)
  const from = earlier ? earlier.from : next.from
  const rest = moves.filter((move) => move !== earlier)
  return from === next.to ? rest : [...rest, { ...next, from }]
}

/** The tree as it would be once the staged changes are applied. */
export function applyMoves(departments: Department[], teams: Team[], moves: OrgMove[]): { departments: Department[]; teams: Team[] } {
  const parent = new Map(moves.filter((move) => move.kind === 'department').map((move) => [move.id, move.to]))
  const placed = new Map(moves.filter((move) => move.kind === 'team').map((move) => [move.id, move.to]))
  return {
    departments: departments.map((department) => (parent.has(department.id) ? { ...department, parentId: parent.get(department.id) ?? null } : department)),
    teams: teams.map((team) => (placed.has(team.id) ? { ...team, departmentId: placed.get(team.id) ?? null } : team)),
  }
}

/** Active people who are on at least one of the Teams, each counted once however many of those Teams they are on. */
export function distinctPeople(people: WorkspacePerson[], teamIds: ReadonlySet<string>): number {
  return people.filter((person) => person.state === 'ACTIVE' && person.teams.some((team) => teamIds.has(team.id))).length
}

/** Teams below a department (any depth) and the distinct people on them. */
export function branchCounts(departments: Department[], teams: Team[], people: WorkspacePerson[], departmentId: string) {
  const inside = subtreeIds(departments, departmentId)
  const here = teams.filter((team) => inside.has(team.departmentId ?? ''))
  return { teams: here.length, people: distinctPeople(people, new Set(here.map((team) => team.id))) }
}

/** The path of department names above a Team, top first, for example "Product / Dev". */
export function departmentPath(departments: Department[], departmentId: string | null): string[] {
  const names: string[] = []
  const seen = new Set<string>()
  let current = departmentId
  while (current && !seen.has(current)) {
    seen.add(current)
    const department = departments.find((item) => item.id === current)
    if (!department) break
    names.unshift(department.name)
    current = department.parentId
  }
  return names
}

/**
 * Narrows the tree to what matches the text: a department or Team by name, a Team by its description, or a Team whose
 * member matches by name or email. A department that matches keeps all of its Teams. Without text everything is kept.
 */
export function matchOrg(
  query: string,
  departments: Department[],
  teams: Team[],
  people: WorkspacePerson[],
): { departmentIds: Set<string>; teamIds: Set<string> } {
  const needle = query.trim().toLowerCase()
  if (needle === '') return { departmentIds: new Set(departments.map((item) => item.id)), teamIds: new Set(teams.map((item) => item.id)) }
  const has = (text: string | null | undefined) => (text ?? '').toLowerCase().includes(needle)
  const matchingPeople = people.filter((person) => has(person.displayName) || has(person.email))
  const personTeams = new Set(matchingPeople.flatMap((person) => person.teams.map((team) => team.id)))
  const matchedDepartments = new Set(departments.filter((department) => has(department.name)).map((department) => department.id))
  // Everything below a matching department matches with it.
  const coveredDepartments = new Set([...matchedDepartments].flatMap((id) => [...subtreeIds(departments, id)]))
  const teamIds = new Set(
    teams
      .filter((team) => has(team.name) || has(team.description) || personTeams.has(team.id) || coveredDepartments.has(team.departmentId ?? ''))
      .map((team) => team.id),
  )
  // The departments above a matching Team stay so that the Team can be seen in place.
  const departmentIds = new Set<string>(coveredDepartments)
  for (const team of teams) {
    if (!teamIds.has(team.id)) continue
    let current = team.departmentId ?? null
    while (current && !departmentIds.has(current)) {
      departmentIds.add(current)
      current = departments.find((item) => item.id === current)?.parentId ?? null
    }
  }
  return { departmentIds, teamIds }
}
