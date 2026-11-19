/** The server's `require_label` rule, shared by every name it checks that way: trimmed, 1 to 200 characters, no control characters. */
export const LABEL_MAX = 200

export type LabelProblem = 'blank' | 'tooLong' | 'control'

export function labelProblem(value: string): LabelProblem | null {
  const trimmed = value.trim()
  if (trimmed === '') return 'blank'
  if ([...trimmed].length > LABEL_MAX) return 'tooLong'
  if (/\p{Cc}/u.test(trimmed)) return 'control'
  return null
}
