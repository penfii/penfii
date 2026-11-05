import { describe, expect, it } from 'vitest'
import { estimateAsChange, estimateHundredths, formatEstimate, formatMinutes, isEstimateText, normalizeEstimateText, scheduleAsChange, scheduleParts, splitMinutes } from './planValues'

describe('estimate text', () => {
  it('accepts a plain decimal with at most two decimals', () => {
    for (const text of ['0', '5', '5.5', '5.25', '007', ' 8 ', '99999999.99']) expect(isEstimateText(text), text).toBe(true)
    for (const text of ['', '-1', '+1', '1e3', '5.123', '.5', '5.', '1,5', 'abc', '123456789']) expect(isEstimateText(text), text).toBe(false)
  })

  it('keeps the shortest form and compares in hundredths', () => {
    expect(normalizeEstimateText('5.00')).toBe('5')
    expect(normalizeEstimateText('0.50')).toBe('0.5')
    expect(normalizeEstimateText('007.10')).toBe('7.1')
    expect(normalizeEstimateText('0')).toBe('0')
    expect(estimateHundredths('0.5')).toBe(50)
    expect(estimateHundredths('12.25')).toBe(1225)
    expect(estimateHundredths('3')).toBe(300)
    expect(formatEstimate({ value: '0.5', unit: 'HOURS' })).toBe('0.5 h')
    expect(formatEstimate({ value: '5', unit: 'POINTS' })).toBe('5 pt')
  })
})

describe('working minutes', () => {
  const labels = { hour: 'h', minute: 'min' }
  it('are written in hours and minutes', () => {
    expect(splitMinutes(90)).toEqual({ hours: 1, minutes: 30 })
    expect(formatMinutes(480, labels)).toBe('8h')
    expect(formatMinutes(90, labels)).toBe('1h 30min')
    expect(formatMinutes(45, labels)).toBe('45min')
    expect(formatMinutes(0, labels)).toBe('0min')
  })
})

describe('schedule as a change', () => {
  it('has the shape the server gives a revision change and reads back', () => {
    const parts = scheduleAsChange({ plannedStartAt: '2026-11-02T00:00:00Z', durationMinutes: 480, isScheduleMilestone: false })
    expect(parts).toEqual(['2026-11-02T00:00:00Z', '480', ''])
    expect(scheduleParts(parts)).toEqual({ start: '2026-11-02T00:00:00Z', duration: '480', milestone: false })
    expect(scheduleParts(scheduleAsChange({ isScheduleMilestone: true, durationMinutes: 0 }))?.milestone).toBe(true)
    expect(scheduleAsChange(null)).toBeNull()
    expect(scheduleParts(null)).toBeNull()
    expect(estimateAsChange({ value: '5' }, 'POINTS')).toEqual(['5', 'POINTS'])
    expect(estimateAsChange(null, 'POINTS')).toBeNull()
  })
})
