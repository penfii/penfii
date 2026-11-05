export type ReadState = {
  generation: number | null
  readThroughSeq: number
  manualUnread: boolean
  manualUnreadFromSeq: number | null
}

export function initialReadState(input: {
  generation: number | null
  readThroughSeq: number | null
  manualUnreadFromSeq?: number | null
}): ReadState {
  return {
    generation: input.generation,
    readThroughSeq: input.readThroughSeq ?? 0,
    manualUnread: input.manualUnreadFromSeq != null,
    manualUnreadFromSeq: input.manualUnreadFromSeq ?? null,
  }
}

export function applyManualUnread(state: ReadState, fromSeq: number): ReadState {
  return {
    ...state,
    manualUnread: true,
    manualUnreadFromSeq: fromSeq,
  }
}

export function clearManualUnread(state: ReadState): ReadState {
  return {
    ...state,
    manualUnread: false,
    manualUnreadFromSeq: null,
  }
}

export function applyStaleRead(state: ReadState, generation: number): ReadState {
  return {
    generation,
    readThroughSeq: state.readThroughSeq,
    manualUnread: state.manualUnread,
    manualUnreadFromSeq: state.manualUnreadFromSeq,
  }
}

export function applyAutoReadSuccess(
  _state: ReadState,
  input: { generation: number; readThroughSeq: number },
): ReadState {
  return {
    generation: input.generation,
    readThroughSeq: input.readThroughSeq,
    manualUnread: false,
    manualUnreadFromSeq: null,
  }
}

function strictReadThrough(
  readThroughSeq: number,
  visibleSeqs: readonly number[],
  ownSeqs: readonly number[],
): number | null {
  const candidates = [...new Set([...visibleSeqs, ...ownSeqs])].sort((left, right) => left - right)
  let cursor = readThroughSeq
  for (const seq of candidates) {
    if (seq <= cursor) continue
    if (seq !== cursor + 1) break
    cursor = seq
  }
  return cursor === readThroughSeq ? null : cursor
}

export function nextReadThroughSeq(
  readThroughSeq: number,
  visibleSeqs: readonly number[],
  ownSeqs: readonly number[] = [],
  options?: { loadedSeqs?: readonly number[]; hasOlder?: boolean },
): number | null {
  const loaded = options?.loadedSeqs
  if (!loaded || loaded.length === 0) return strictReadThrough(readThroughSeq, visibleSeqs, ownSeqs)
  const seen = new Set([...visibleSeqs, ...ownSeqs])
  // Having the newest message on screen counts as reading the conversation up to it.
  const newest = Math.max(...loaded)
  if (newest > readThroughSeq && seen.has(newest)) return newest
  const first = Math.min(...loaded)
  if (options?.hasOlder && first > readThroughSeq + 1) return strictReadThrough(readThroughSeq, visibleSeqs, ownSeqs)
  const points = [...new Set([...loaded, ...ownSeqs])].sort((left, right) => left - right)
  let cursor = readThroughSeq
  for (const seq of points) {
    if (seq <= cursor) continue
    if (!seen.has(seq)) break
    cursor = seq
  }
  return cursor === readThroughSeq ? null : cursor
}
