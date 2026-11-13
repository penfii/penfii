/**
 * `openapi.json` is the public description of the API (the server keeps the original in
 * `server/api/openapi.json`, and `npm run contract:sync` copies it here). The types in `index.ts` are what the
 * client works with, so they are held to that description: a type named like a schema may not claim a
 * field the API does not describe, may not call a field optional that the API requires, and must agree
 * with it on the kind of each value and on whether it can be null.
 *
 * A type may have fewer fields than the schema (the client ignores what it does not use). A field the
 * client adds on purpose (derived in the api client, or filled in when the server omits it) is listed
 * in `CLIENT_FIELDS` with the reason, so a new one cannot slip in unnoticed.
 */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'
import { describe, expect, it } from 'vitest'

const here = path.dirname(fileURLToPath(import.meta.url))
const openapi = JSON.parse(fs.readFileSync(path.join(here, '..', 'openapi.json'), 'utf8')) as {
  components: { schemas: Record<string, Schema> }
}
const source = ts.createSourceFile('index.ts', fs.readFileSync(path.join(here, 'index.ts'), 'utf8'), ts.ScriptTarget.Latest, true)

type Schema = {
  type?: string | string[]
  properties?: Record<string, Schema>
  required?: string[]
  $ref?: string
  enum?: unknown[]
  items?: Schema
}

/** Fields of a client type that the API does not describe, and why. */
const CLIENT_FIELDS: Record<string, string> = {
  'Channel.joined': 'derived from the server role / membership list by the api client',
  'Channel.conversationId': 'falls back to the id when the server omits it (the id is the conversation)',
  'Conversation.generation': 'flattened from conversation.read by the api client',
  'Conversation.readThroughSeq': 'flattened from conversation.read by the api client',
  'Conversation.manualUnreadFromSeq': 'flattened from conversation.read by the api client',
  'Conversation.title': 'taken from the channel name by the api client',
  'Reaction.count': 'userIds.length, computed by the api client',
  'Reaction.reactedByMe': 'computed from userIds and the signed-in person by the api client',
  'MessageVersion.editedAt': 'renamed from recordedAt by the api client',
}

/** Fields the API may send as null that the client type does not allow, because the api client replaces the null. */
const NORMALIZED_NULLS: Record<string, string> = {
  'ConversationChange.messageVersion': 'a null version becomes 0 in the api client (feed and socket frames)',
}

type Member = { name: string; optional: boolean; type: ts.TypeNode }
const declared = new Map<string, Member[]>()
const aliases = new Map<string, ts.TypeNode>()
source.forEachChild((node) => {
  if (ts.isTypeAliasDeclaration(node)) {
    aliases.set(node.name.text, node.type)
    if (ts.isTypeLiteralNode(node.type)) declared.set(node.name.text, members(node.type.members))
  } else if (ts.isInterfaceDeclaration(node)) {
    declared.set(node.name.text, members(node.members))
  }
})

function members(list: ts.NodeArray<ts.TypeElement>): Member[] {
  return list.filter(ts.isPropertySignature).map((member) => ({
    name: member.name.getText(source).replace(/^['"]|['"]$/g, ''),
    optional: member.questionToken !== undefined,
    type: member.type ?? ts.factory.createKeywordTypeNode(ts.SyntaxKind.UnknownKeyword),
  }))
}

type Kinds = { kinds: Set<string>; nullable: boolean; any: boolean }

/** The JSON kinds a TypeScript type can hold. */
function kindsOf(node: ts.TypeNode, depth = 0): Kinds {
  const out: Kinds = { kinds: new Set(), nullable: false, any: false }
  const add = (kind: string) => out.kinds.add(kind)
  const visit = (type: ts.TypeNode) => {
    if (ts.isParenthesizedTypeNode(type)) return visit(type.type)
    if (ts.isUnionTypeNode(type)) return type.types.forEach(visit)
    if (ts.isLiteralTypeNode(type)) {
      if (type.literal.kind === ts.SyntaxKind.NullKeyword) out.nullable = true
      else if (ts.isStringLiteral(type.literal)) add('string')
      else if (ts.isNumericLiteral(type.literal)) add('number')
      else add('boolean')
      return
    }
    // Only a few of the hundreds of syntax kinds mean a JSON kind here; the rest fall through to the checks below.
    // eslint-disable-next-line @typescript-eslint/switch-exhaustiveness-check
    switch (type.kind) {
      case ts.SyntaxKind.StringKeyword:
        return add('string')
      case ts.SyntaxKind.NumberKeyword:
        return add('number')
      case ts.SyntaxKind.BooleanKeyword:
        return add('boolean')
      case ts.SyntaxKind.NullKeyword:
        out.nullable = true
        return
      case ts.SyntaxKind.UndefinedKeyword:
      case ts.SyntaxKind.VoidKeyword:
        return
      case ts.SyntaxKind.UnknownKeyword:
      case ts.SyntaxKind.AnyKeyword:
        out.any = true
        return
      default:
        break
    }
    if (ts.isArrayTypeNode(type) || ts.isTupleTypeNode(type)) return add('array')
    if (ts.isTypeLiteralNode(type) || ts.isMappedTypeNode(type)) return add('object')
    if (ts.isTypeReferenceNode(type)) {
      const name = type.typeName.getText(source)
      if (name === 'Array' || name === 'ReadonlyArray') return add('array')
      if (name === 'Record' || name === 'Partial') return add('object')
      const target = aliases.get(name)
      if (target && depth < 4) {
        const inner = kindsOf(target, depth + 1)
        inner.kinds.forEach(add)
        if (inner.nullable) out.nullable = true
        if (inner.any) out.any = true
        return
      }
      return add('object')
    }
    if (ts.isIntersectionTypeNode(type)) return add('object')
    out.any = true
  }
  visit(node)
  return out
}

function documentedKinds(schema: Schema | undefined): Kinds | null {
  if (!schema) return null
  if (schema.$ref) return { kinds: new Set(['object']), nullable: false, any: false }
  const types = Array.isArray(schema.type) ? schema.type : schema.type ? [schema.type] : schema.enum ? ['string'] : []
  if (types.length === 0) return null
  return {
    kinds: new Set(types.filter((t) => t !== 'null').map((t) => (t === 'integer' ? 'number' : t))),
    nullable: types.includes('null'),
    any: false,
  }
}

function findProblems(schemas: Record<string, Schema>): string[] {
  const problems: string[] = []
  for (const name of Object.keys(schemas).filter((n) => declared.has(n))) {
    const schema = schemas[name]!
    const properties = schema.properties ?? {}
    const required = new Set(schema.required ?? [])
    for (const member of declared.get(name)!) {
      const at = `${name}.${member.name}`
      if (!(member.name in properties)) {
        if (!(at in CLIENT_FIELDS)) problems.push(`${at}: the client expects it, openapi.json does not describe it`)
        continue
      }
      if (member.optional && required.has(member.name)) problems.push(`${at}: optional in the client, required by openapi.json`)
      const documented = documentedKinds(properties[member.name])
      if (!documented) continue
      const client = kindsOf(member.type)
      if (!client.any) {
        for (const kind of client.kinds) {
          if (!documented.kinds.has(kind)) problems.push(`${at}: the client reads ${kind}, openapi.json says ${[...documented.kinds].join('|')}`)
        }
      }
      if (documented.nullable && !client.nullable && !client.any && !member.optional && !(at in NORMALIZED_NULLS)) {
        problems.push(`${at}: openapi.json allows null, the client type does not`)
      }
    }
  }
  return problems
}

describe('the client types agree with openapi.json', () => {
  const schemas = openapi.components.schemas
  const shared = [...declared.keys()].filter((name) => schemas[name])

  it('compares a meaningful number of types', () => {
    expect(shared.length).toBeGreaterThan(40)
  })

  it('claims no field the API does not describe, no optional field it requires, and reads each value as the kind it is', () => {
    expect(findProblems(schemas)).toEqual([])
  })

  it('notices drift: a field the API stopped describing, one it started requiring, a changed kind', () => {
    const drifted = structuredClone(schemas)
    delete drifted.Workspace!.properties!.timeZone
    drifted.Team!.properties!.name = { type: 'integer' }
    const found = findProblems(drifted).join('\n')
    expect(found).toContain('Workspace.timeZone: the client expects it')
    expect(found).toContain('Team.name: the client reads string, openapi.json says number')
    // `name` is required and not optional in the client, so only a client-optional field shows the second rule.
    const optional = structuredClone(schemas)
    const member = declared.get('Project')!.find((m) => m.optional)!
    optional.Project!.required = [member.name]
    expect(findProblems(optional).join('\n')).toContain(`Project.${member.name}: optional in the client, required by openapi.json`)
  })

  it('lists no exception that is not needed any more', () => {
    for (const key of Object.keys(NORMALIZED_NULLS)) {
      const [type, field] = key.split('.') as [string, string]
      const member = declared.get(type)?.find((m) => m.name === field)
      expect(member, key).toBeDefined()
      expect(kindsOf(member!.type).nullable, `${key} allows null now, remove it from NORMALIZED_NULLS`).toBe(false)
    }

    for (const key of Object.keys(CLIENT_FIELDS)) {
      const [type, field] = key.split('.') as [string, string]
      expect(declared.get(type)?.some((member) => member.name === field), key).toBe(true)
      expect(schemas[type]?.properties?.[field], `${key} is described now, remove it from CLIENT_FIELDS`).toBeUndefined()
    }
  })
})
