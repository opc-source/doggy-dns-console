import type { InvestigationFilter } from '../api/types'

/** Mirrors `MIN_RANGE_MINUTES` in `src/domain.rs`. */
export const MIN_RANGE_MINUTES = 5
/** Mirrors `MAX_RANGE_MINUTES` in `src/domain.rs` — the investigation ceiling. */
export const MAX_RANGE_MINUTES = 7 * 1440
/** Mirrors `FUTURE_TOLERANCE_MS` in `src/domain.rs`: the browser clock may run slightly ahead. */
export const FUTURE_TOLERANCE_MS = 60_000
/** Mirrors `DEFAULT_WINDOW_MINUTES` in `src/domain.rs`. */
export const DEFAULT_WINDOW_MINUTES = 60
const MINUTE = 60_000

/** What the filter bar holds. Absolute once both `fromMs` and `toMs` are set. */
export interface FilterDraft {
  fromMs: number | null
  toMs: number | null
  windowMinutes: number | null
  domain: string
  responseCode: string
  instance: string
}

export interface Bracket {
  startMs: number
  endMs: number
}

export function emptyDraft(): FilterDraft {
  return {
    fromMs: null,
    toMs: null,
    windowMinutes: null,
    domain: '',
    responseCode: '',
    instance: '',
  }
}

export function relativeDraft(windowMinutes: number): FilterDraft {
  return { ...emptyDraft(), windowMinutes }
}

export function isAbsolute(draft: FilterDraft): boolean {
  return draft.fromMs !== null && draft.toMs !== null
}

/**
 * A draft with exactly one absolute bound is not a bracket the operator picked:
 * a lone `to` anchors a relative window to a frozen instant, and a lone `from`
 * only means "everything up to the server clock". The bar always completes the
 * pair, so this only refuses hand-edited URLs — loudly, instead of silently
 * querying something other than what the URL appears to say.
 */
export function validateDraft(draft: FilterDraft): string | null {
  if ((draft.fromMs === null) !== (draft.toMs === null)) {
    return '区间需要同时给出起止时间'
  }
  return null
}

type RawQuery = Record<string, string | string[] | null | undefined>

function firstSingle(value: string | string[] | null | undefined): string | null {
  if (value === null || value === undefined) return null
  const text = Array.isArray(value) ? (value[0] ?? null) : value
  return text === '' ? null : text
}

/**
 * The backend's integer grammar, per field: `from`/`to` deserialise as
 * `Option<i64>` (signed) but `FailureQuery.window_minutes` in
 * `src/api/failures.rs` is `Option<u32>`, so a negative window is a 400
 * there and must be pre-rejected here — the grammar follows each field's
 * Rust type, because this module is one contract with `src/domain.rs`, not
 * two. Hex, exponent, fraction and padded text fail both grammars; a
 * leading minus fails only the unsigned one. Anything dropped falls back
 * to the default relative window instead of a guessed value.
 */
function parseStrictInt(value: string | null, unsigned = false): number | null {
  if (value === null || !(unsigned ? /^\d+$/ : /^-?\d+$/).test(value)) return null
  return Number(value)
}

export function parseFilterQuery(query: RawQuery): FilterDraft {
  return {
    fromMs: parseStrictInt(firstSingle(query.from)),
    toMs: parseStrictInt(firstSingle(query.to)),
    windowMinutes: parseStrictInt(firstSingle(query.windowMinutes), true),
    domain: firstSingle(query.domain) ?? '',
    responseCode: firstSingle(query.responseCode) ?? '',
    instance: firstSingle(query.instance) ?? '',
  }
}

/**
 * The URL is the only state source, so the shape it writes matters: relative
 * drafts travel as `windowMinutes` (the server owns "now"), absolute drafts as
 * `from`/`to`. Mixing both would make the URL drift on every refresh.
 */
export function toRouteQuery(draft: FilterDraft): Record<string, string> {
  const query: Record<string, string> = {}
  if (isAbsolute(draft)) {
    query.from = String(draft.fromMs)
    query.to = String(draft.toMs)
  } else if (draft.windowMinutes !== null) {
    query.windowMinutes = String(draft.windowMinutes)
  }
  if (draft.domain) query.domain = draft.domain
  if (draft.responseCode) query.responseCode = draft.responseCode
  if (draft.instance) query.instance = draft.instance
  return query
}

/** Mirrors `Range::resolve` in `src/domain.rs`: an explicit pair wins. */
export function resolveBracket(draft: FilterDraft, nowMs: number): Bracket {
  const endMs = draft.toMs ?? nowMs
  if (draft.fromMs !== null && draft.toMs !== null) return { startMs: draft.fromMs, endMs }
  const minutes = draft.windowMinutes ?? DEFAULT_WINDOW_MINUTES
  return { startMs: draft.fromMs ?? endMs - minutes * MINUTE, endMs }
}

/**
 * The same four rules `Range::checked` in `src/domain.rs` enforces, in the
 * same order — a mirror, not a second opinion. It reports; it never clamps.
 */
export function validateBracket(bracket: Bracket, nowMs: number): string | null {
  if (bracket.startMs >= bracket.endMs) return '区间起始必须早于结束'
  if (bracket.endMs > nowMs + FUTURE_TOLERANCE_MS) {
    return `区间结束不能超过当前时间 ${FUTURE_TOLERANCE_MS / MINUTE} 分钟`
  }
  const spanMs = bracket.endMs - bracket.startMs
  if (spanMs < MIN_RANGE_MINUTES * MINUTE) return `区间不足 ${MIN_RANGE_MINUTES} 分钟`
  if (spanMs > MAX_RANGE_MINUTES * MINUTE) {
    return `区间超过 ${MAX_RANGE_MINUTES} 分钟（7 天）上限，请收窄`
  }
  return null
}

/** Only the params the endpoint reads; an absolute pair suppresses the shortcut. */
export function toRequestQuery(draft: FilterDraft): Record<string, string> {
  const query: Record<string, string> = {}
  if (isAbsolute(draft)) {
    query.from = String(draft.fromMs)
    query.to = String(draft.toMs)
  } else if (draft.windowMinutes !== null) {
    query.windowMinutes = String(draft.windowMinutes)
  }
  if (draft.domain) query.domain = draft.domain
  if (draft.responseCode) query.responseCode = draft.responseCode
  if (draft.instance) query.instance = draft.instance
  return query
}

/** Request shape with numeric bounds; `toRequestQuery` is its string twin. */
export function toInvestigationFilter(draft: FilterDraft, nowMs: number): InvestigationFilter {
  const bracket = resolveBracket(draft, nowMs)
  return {
    from: bracket.startMs,
    to: bracket.endMs,
    domain: draft.domain || null,
    responseCode: draft.responseCode || null,
    instance: draft.instance || null,
  }
}

function pad(value: number): string {
  return String(value).padStart(2, '0')
}

/** `datetime-local` value in the browser's local zone, minute precision. */
export function msToDatetime(ms: number): string {
  const date = new Date(ms)
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

export function datetimeToMs(value: string): number | null {
  if (!value) return null
  const parsed = Date.parse(value)
  if (Number.isNaN(parsed)) return null
  // `Date.parse` silently normalises a non-existent local time (a DST gap) and
  // an impossible date like 2026-02-30. A bound the input cannot re-render must
  // be refused, not shifted by an hour or a day.
  return msToDatetime(parsed) === value ? parsed : null
}

function formatQuantity(value: number): string {
  return Number.isInteger(value) ? String(value) : value.toFixed(1)
}

// `_nowMs` keeps the clock argument uniform across the contract's functions;
// a relative label never needs it.
export function rangeLabel(draft: FilterDraft, _nowMs: number): string {
  if (isAbsolute(draft)) {
    return `${msToDatetime(draft.fromMs as number)} ~ ${msToDatetime(draft.toMs as number)}`
  }
  const minutes = draft.windowMinutes ?? DEFAULT_WINDOW_MINUTES
  if (minutes < 60) return `近 ${minutes} 分钟`
  // 1440 is the live picker's widest option; the hour unit holds through it,
  // so the label only flips to days above the relative class's ceiling.
  if (minutes <= 1440) return `近 ${formatQuantity(minutes / 60)} 小时`
  return `近 ${formatQuantity(minutes / 1440)} 天`
}
