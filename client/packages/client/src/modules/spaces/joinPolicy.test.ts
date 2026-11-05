import { describe, expect, it } from 'vitest'
import { shouldOfferJoin } from './joinPolicy'

describe('shouldOfferJoin', () => {
  it('offers Join in a public channel of no Project or Team the person has not joined', () => {
    expect(shouldOfferJoin({ visibility: 'PUBLIC', joined: false })).toBe(true)
    expect(shouldOfferJoin({ visibility: 'PUBLIC', joined: false, projectId: null, teamId: null })).toBe(true)
  })

  it('offers nothing once joined, in a private channel, or where the Project or Team people write without joining', () => {
    expect(shouldOfferJoin({ visibility: 'PUBLIC', joined: true })).toBe(false)
    expect(shouldOfferJoin({ visibility: 'PUBLIC' })).toBe(false)
    expect(shouldOfferJoin({ visibility: 'PRIVATE', joined: false })).toBe(false)
    expect(shouldOfferJoin({ visibility: 'PUBLIC', joined: false, projectId: 'p1' })).toBe(false)
    expect(shouldOfferJoin({ visibility: 'PUBLIC', joined: false, teamId: 't1' })).toBe(false)
  })
})
