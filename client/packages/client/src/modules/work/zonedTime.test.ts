import { describe, expect, it } from 'vitest'
import { formatZoned, instantToLocalInput, localInputToInstant, zoneOffsetMinutes } from './zonedTime'

describe('wall clock times in another zone', () => {
  it('reads the offset of a zone at an instant, daylight saving included', () => {
    expect(zoneOffsetMinutes(new Date('2026-11-02T00:00:00Z'), 'Asia/Seoul')).toBe(540)
    expect(zoneOffsetMinutes(new Date('2026-01-15T12:00:00Z'), 'America/New_York')).toBe(-300)
    expect(zoneOffsetMinutes(new Date('2026-07-15T12:00:00Z'), 'America/New_York')).toBe(-240)
    expect(zoneOffsetMinutes(new Date('2026-04-05T12:00:00Z'), 'Australia/Lord_Howe')).toBe(630)
  })

  it('turns a typed wall time into the instant it names and back', () => {
    expect(localInputToInstant('2026-11-02T09:00', 'Asia/Seoul')).toBe('2026-11-02T00:00:00Z')
    expect(localInputToInstant('2026-01-15T09:00', 'America/New_York')).toBe('2026-01-15T14:00:00Z')
    expect(localInputToInstant('2026-07-15T09:00', 'America/New_York')).toBe('2026-07-15T13:00:00Z')
    expect(instantToLocalInput('2026-11-02T00:00:00Z', 'Asia/Seoul')).toBe('2026-11-02T09:00')
    for (const [local, zone] of [['2026-11-02T09:00', 'Asia/Seoul'], ['2026-07-15T23:45', 'America/New_York'], ['2026-12-31T00:00', 'Europe/London'], ['2026-04-05T12:30', 'Australia/Lord_Howe']] as const) {
      expect(instantToLocalInput(localInputToInstant(local, zone) ?? '', zone)).toBe(local)
    }
  })

  it('reads a time the clocks skip as the same wall time after the change, and a repeated time as its first occurrence', () => {
    // New York, 2026-03-08: 02:00-03:00 does not exist.
    expect(localInputToInstant('2026-03-08T02:30', 'America/New_York')).toBe('2026-03-08T07:30:00Z')
    expect(instantToLocalInput('2026-03-08T07:30:00Z', 'America/New_York')).toBe('2026-03-08T03:30')
    // New York, 2026-11-01: 01:00-02:00 happens twice; the first is still summer time.
    expect(localInputToInstant('2026-11-01T01:30', 'America/New_York')).toBe('2026-11-01T05:30:00Z')
    // The day before and after are plain.
    expect(localInputToInstant('2026-03-07T02:30', 'America/New_York')).toBe('2026-03-07T07:30:00Z')
    expect(localInputToInstant('2026-03-09T02:30', 'America/New_York')).toBe('2026-03-09T06:30:00Z')
  })

  it('refuses what is not a date and time', () => {
    for (const text of ['', '2026-11-02', '2026-11-02 09:00', '2026-13-02T09:00', '2026-02-30T09:00', 'tomorrow']) {
      expect(localInputToInstant(text, 'Asia/Seoul')).toBeNull()
    }
  })

  it('shows an instant on the wall clock of the zone, in the reader\'s language', () => {
    expect(formatZoned('2026-11-02T00:00:00Z', 'Asia/Seoul', 'en-US')).toContain('9:00')
    expect(formatZoned('2026-11-02T00:00:00Z', 'America/New_York', 'en-US')).toContain('7:00')
  })
})
