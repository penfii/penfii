import { Avatar } from '../../design/ui'

const SHOWN = 3

/** The first few people as overlapping avatars, then "+N" (frames 1.1 and 2.1). */
export function MemberStack({ members }: { members: Array<{ userId: string; displayName: string }> }) {
  if (members.length === 0) return null
  const rest = members.length - SHOWN
  return (
    <span className="member-stack" title={members.map((member) => member.displayName).join(', ')}>
      {members.slice(0, SHOWN).map((member) => (
        <Avatar key={member.userId} name={member.displayName} userId={member.userId} />
      ))}
      {rest > 0 ? <span className="member-more">+{rest}</span> : null}
    </span>
  )
}
