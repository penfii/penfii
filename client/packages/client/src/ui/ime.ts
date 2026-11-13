/**
 * True while a key press belongs to an input method (Korean, Japanese, Chinese ...) that is still
 * composing text. The Enter that picks a candidate is part of the composition and must not also submit,
 * open or confirm anything. `isComposing` covers most browsers; Safari delivers that last keydown just
 * after the composition ended, with `isComposing` false and the legacy `keyCode` 229, so both are read.
 */
export function isImeKey(event: { isComposing?: boolean; keyCode?: number; nativeEvent?: { isComposing?: boolean; keyCode?: number } }): boolean {
  return Boolean(event.isComposing || event.nativeEvent?.isComposing || event.keyCode === 229 || event.nativeEvent?.keyCode === 229)
}
