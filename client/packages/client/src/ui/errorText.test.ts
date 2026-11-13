import { ApiError, isDbUnavailable } from '@penfii/api-client'
import { describe, expect, it } from 'vitest'

import { errorText } from './errorText'

describe('db_unavailable', () => {
  const unavailable = new ApiError(503, 'db_unavailable', 'the database is temporarily unavailable')

  it('recognises only that code on 503', () => {
    expect(isDbUnavailable(unavailable)).toBe(true)
    expect(isDbUnavailable(new ApiError(503, 'search_unavailable', 'search is down'))).toBe(false)
    expect(isDbUnavailable(new ApiError(500, 'db_unavailable', 'the database is temporarily unavailable'))).toBe(false)
    expect(isDbUnavailable(new Error('no'))).toBe(false)
  })

  it('shows the temporary-unavailable text and not the server sentence', () => {
    expect(errorText(unavailable, 'generic', 'try again')).toBe('try again')
    expect(errorText(new ApiError(503, 'search_unavailable', 'search is down'), 'generic', 'try again')).toBe('search is down')
  })
})
