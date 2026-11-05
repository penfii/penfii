import { describe, expect, it } from 'vitest'
import type { Department, Team, WorkspacePerson } from '@penfii/contracts'
import { applyMoves, branchCounts, departmentPath, distinctPeople, matchOrg, stageMove, subtreeIds, type OrgMove } from './orgTree'

const department = (id: string, name: string, parentId: string | null): Department => ({ id, workspaceId: 'w', parentId, name, position: 0 })
const team = (id: string, name: string, departmentId: string | null, extra: Partial<Team> = {}): Team => ({ id, name, archivedAt: null, description: null, departmentId, ...extra })
const person = (userId: string, name: string, teams: string[], state = 'ACTIVE'): WorkspacePerson =>
  ({ userId, displayName: name, email: `${name.toLowerCase()}@example.com`, hasAvatar: false, role: 'MEMBER', state, teams: teams.map((id) => ({ id, name: id })) }) as WorkspacePerson

const departments = [department('prod', 'Product', null), department('dev', 'Dev', 'prod'), department('design', 'Design', null)]
const teams = [team('p', 'Product', 'prod'), team('b', 'Backend', 'dev'), team('f', 'Frontend', 'dev'), team('d', 'Design', 'design'), team('x', 'Loose', null)]
const people = [person('1', 'Ann', ['b', 'f']), person('2', 'Bob', ['p']), person('3', 'Cy', []), person('4', 'Di', ['b'], 'INACTIVE')]

describe('org tree helpers', () => {
  it('finds a department with everything below it', () => {
    expect([...subtreeIds(departments, 'prod')].sort()).toEqual(['dev', 'prod'])
  })
  it('counts a person once however many Teams they are on, and skips inactive people', () => {
    expect(distinctPeople(people, new Set(['b', 'f']))).toBe(1)
    expect(branchCounts(departments, teams, people, 'prod')).toEqual({ teams: 3, people: 2 })
  })
  it('stages, replaces and drops changes', () => {
    const first: OrgMove = { kind: 'team', id: 'f', name: 'Frontend', from: 'dev', to: 'design' }
    let moves = stageMove([], first)
    expect(moves).toHaveLength(1)
    moves = stageMove(moves, { ...first, from: 'design', to: 'prod' })
    expect(moves).toEqual([{ ...first, from: 'dev', to: 'prod' }])
    expect(stageMove(moves, { ...first, from: 'prod', to: 'dev' })).toEqual([])
  })
  it('shows the tree as it would be after the changes', () => {
    const applied = applyMoves(departments, teams, [
      { kind: 'team', id: 'f', name: 'Frontend', from: 'dev', to: 'design' },
      { kind: 'department', id: 'dev', name: 'Dev', from: 'prod', to: null },
    ])
    expect(applied.teams.find((item) => item.id === 'f')?.departmentId).toBe('design')
    expect(applied.departments.find((item) => item.id === 'dev')?.parentId).toBeNull()
  })
  it('names the departments above a Team', () => {
    expect(departmentPath(departments, 'dev')).toEqual(['Product', 'Dev'])
    expect(departmentPath(departments, null)).toEqual([])
  })
  it('matches by Team, department, description or member and keeps the way down to a hit', () => {
    expect(matchOrg('', departments, teams, people).teamIds.size).toBe(5)
    const byTeam = matchOrg('front', departments, teams, people)
    expect([...byTeam.teamIds]).toEqual(['f'])
    expect([...byTeam.departmentIds].sort()).toEqual(['dev', 'prod'])
    expect([...matchOrg('product', departments, teams, people).teamIds].sort()).toEqual(['b', 'f', 'p'])
    expect([...matchOrg('bob@', departments, teams, people).teamIds]).toEqual(['p'])
    expect(matchOrg('zzz', departments, teams, people).teamIds.size).toBe(0)
  })
})
