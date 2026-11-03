import { describe, expect, it } from 'vitest'
import {
  FUTURE_TOLERANCE_MS,
  MAX_RANGE_MINUTES,
  MIN_RANGE_MINUTES,
  datetimeToMs,
  emptyDraft,
  isAbsolute,
  msToDatetime,
  parseFilterQuery,
  rangeLabel,
  relativeDraft,
  resolveBracket,
  toInvestigationFilter,
  toRequestQuery,
  toRouteQuery,
  validateBracket,
} from '../failureFilter'

const NOW = 1_761_436_800_000
const HOUR = 3_600_000

describe('resolveBracket', () => {
  it('lets from/to win over windowMinutes', () => {
    const draft = { ...emptyDraft(), fromMs: NOW - 10 * HOUR, toMs: NOW, windowMinutes: 60 }
    expect(resolveBracket(draft, NOW)).toEqual({ startMs: NOW - 10 * HOUR, endMs: NOW })
  })

  it('expands a relative window backwards from to', () => {
    const draft = { ...emptyDraft(), toMs: NOW, windowMinutes: 120 }
    expect(resolveBracket(draft, NOW)).toEqual({ startMs: NOW - 2 * HOUR, endMs: NOW })
  })

  it('defaults to a relative hour anchored at now', () => {
    const draft = emptyDraft()
    expect(resolveBracket(draft, NOW)).toEqual({ startMs: NOW - HOUR, endMs: NOW })
  })

  it('keeps a lone from bound and derives the end from the window', () => {
    const draft = { ...emptyDraft(), fromMs: NOW - 3 * HOUR, toMs: null, windowMinutes: null }
    expect(resolveBracket(draft, NOW)).toEqual({ startMs: NOW - 3 * HOUR, endMs: NOW })
  })
})

describe('validateBracket', () => {
  it('accepts the legal edges', () => {
    expect(
      validateBracket({ startMs: NOW - MIN_RANGE_MINUTES * 60_000, endMs: NOW }, NOW),
    ).toBeNull()
    expect(
      validateBracket({ startMs: NOW - MAX_RANGE_MINUTES * 60_000, endMs: NOW }, NOW),
    ).toBeNull()
    expect(validateBracket({ startMs: NOW - HOUR, endMs: NOW + FUTURE_TOLERANCE_MS }, NOW)).toBeNull()
  })

  it('rejects the four documented rules', () => {
    expect(validateBracket({ startMs: NOW, endMs: NOW }, NOW)).toMatch(/早于/)
    expect(validateBracket({ startMs: NOW + HOUR, endMs: NOW }, NOW)).toMatch(/早于/)
    expect(validateBracket({ startMs: NOW - 4 * 60_000, endMs: NOW }, NOW)).toMatch(/5/)
    expect(
      validateBracket({ startMs: NOW - MAX_RANGE_MINUTES * 60_000 - 1, endMs: NOW }, NOW),
    ).toMatch(/10080/)
    expect(
      validateBracket({ startMs: NOW - HOUR, endMs: NOW + FUTURE_TOLERANCE_MS + 1 }, NOW),
    ).toMatch(/不能超过当前时间/)
  })

  it('never clamps a 7-day bracket back to a day', () => {
    const bracket = { startMs: NOW - MAX_RANGE_MINUTES * 60_000, endMs: NOW }
    expect(validateBracket(bracket, NOW)).toBeNull()
    expect(bracket.endMs - bracket.startMs).toBe(MAX_RANGE_MINUTES * 60_000)
  })
})

describe('query round-trip', () => {
  it('parses the documented list URL', () => {
    const draft = parseFilterQuery({
      from: String(NOW - 3 * HOUR),
      to: String(NOW),
      domain: 'nacos',
      responseCode: 'SERVFAIL',
      instance: 'dns-node-2',
    })
    expect(draft.fromMs).toBe(NOW - 3 * HOUR)
    expect(draft.toMs).toBe(NOW)
    expect(draft.domain).toBe('nacos')
    expect(draft.responseCode).toBe('SERVFAIL')
    expect(draft.instance).toBe('dns-node-2')
    expect(isAbsolute(draft)).toBe(true)
  })

  it('parses a relative shortcut', () => {
    const draft = parseFilterQuery({ windowMinutes: '1440' })
    expect(draft.windowMinutes).toBe(1440)
    expect(isAbsolute(draft)).toBe(false)
  })

  it('drops non-numeric bounds instead of guessing', () => {
    const draft = parseFilterQuery({ from: 'abc', to: 'def' })
    expect(draft.fromMs).toBeNull()
    expect(draft.toMs).toBeNull()
  })

  // The backend deserialises from/to as i64 and windowMinutes as u32, so only
  // the strict integer grammar survives; JavaScript coercions (fractions, hex,
  // exponents, padded values) must be dropped, never guessed.
  it.each([
    ['a fractional value', '1.5'],
    ['a hexadecimal value', '0x10'],
    ['an exponent value', '1e3'],
    ['a padded value', ' 360 '],
  ])('drops %s instead of coercing it', (_label, value) => {
    const draft = parseFilterQuery({ from: value, to: value, windowMinutes: value })
    expect(draft.fromMs).toBeNull()
    expect(draft.toMs).toBeNull()
    expect(draft.windowMinutes).toBeNull()
  })

  it('keeps plain integers even when the sibling bound is garbage', () => {
    const draft = parseFilterQuery({ from: String(NOW - HOUR), to: 'abc' })
    expect(draft.fromMs).toBe(NOW - HOUR)
    expect(draft.toMs).toBeNull()
    // A one-digit integer is legal backend grammar; validation rejects the
    // bracket later — the parser never drops a well-formed value on sight.
    expect(parseFilterQuery({ from: '1' }).fromMs).toBe(1)
  })

  // The grammars must genuinely differ per Rust type: `window_minutes` is
  // `Option<u32>` in `FailureQuery` (`src/api/failures.rs`), so a negative
  // window is rejected by the backend and pre-rejected here, while `from`/
  // `to` stay `Option<i64>` and keep accepting a leading minus.
  it('rejects a negative windowMinutes but still parses negative from/to', () => {
    expect(parseFilterQuery({ windowMinutes: '-5' }).windowMinutes).toBeNull()
    const signed = parseFilterQuery({ from: '-5', to: '-5' })
    expect(signed.fromMs).toBe(-5)
    expect(signed.toMs).toBe(-5)
  })

  it('degrades a tampered negative windowMinutes to the default relative hour', () => {
    const draft = parseFilterQuery({ windowMinutes: '-5' })
    expect(resolveBracket(draft, NOW)).toEqual({ startMs: NOW - HOUR, endMs: NOW })
    // Delivered behaviour for a null window with no absolute bounds: the
    // request query emits neither windowMinutes nor from/to, so the server
    // applies its own default — a '-5' never reaches the wire.
    expect(toRequestQuery(draft)).toEqual({})
  })

  it('writes a relative draft as windowMinutes only', () => {
    const query = toRouteQuery(relativeDraft(60))
    expect(query).toEqual({ windowMinutes: '60' })
  })

  it('writes an absolute draft as from/to only', () => {
    const draft = { ...emptyDraft(), fromMs: NOW - HOUR, toMs: NOW, windowMinutes: null }
    expect(toRouteQuery(draft)).toEqual({ from: String(NOW - HOUR), to: String(NOW) })
  })

  it('round-trips through the URL', () => {
    const draft = {
      ...emptyDraft(),
      fromMs: NOW - 26 * HOUR,
      toMs: NOW,
      windowMinutes: null,
      instance: 'dns-node-2',
    }
    expect(parseFilterQuery(toRouteQuery(draft))).toEqual(draft)
  })

  it('sends only the params the backend reads', () => {
    const draft = {
      ...emptyDraft(),
      fromMs: NOW - 2 * HOUR,
      toMs: NOW,
      windowMinutes: 60,
      instance: 'dns-node-2',
    }
    const search = toRequestQuery(draft, { limit: 200 })
    expect(search).toEqual({
      from: String(NOW - 2 * HOUR),
      to: String(NOW),
      instance: 'dns-node-2',
      limit: '200',
    })
  })
})

describe('rangeLabel', () => {
  it('names a relative hour', () => {
    expect(rangeLabel(relativeDraft(60), NOW)).toBe('近 1 小时')
    expect(rangeLabel(relativeDraft(30), NOW)).toBe('近 30 分钟')
    // 1440 is the widest option the live picker offers; its label must not
    // flip to a different unit at the value the picker itself shows.
    expect(rangeLabel(relativeDraft(1440), NOW)).toBe('近 24 小时')
  })

  it('names relative days only above the live-class ceiling', () => {
    expect(rangeLabel(relativeDraft(2880), NOW)).toBe('近 2 天')
    expect(rangeLabel(relativeDraft(MAX_RANGE_MINUTES), NOW)).toBe('近 7 天')
  })

  it('names an absolute bracket in local time', () => {
    const draft = { ...emptyDraft(), fromMs: NOW - 3 * HOUR, toMs: NOW, windowMinutes: null }
    expect(rangeLabel(draft, NOW)).toBe(`${msToDatetime(NOW - 3 * HOUR)} ~ ${msToDatetime(NOW)}`)
  })
})

describe('datetimeToMs', () => {
  it('round-trips through the datetime-local control', () => {
    const text = msToDatetime(NOW)
    expect(datetimeToMs(text)).toBe(NOW - (NOW % 60_000))
  })

  it('rejects blank and malformed input', () => {
    expect(datetimeToMs('')).toBeNull()
    expect(datetimeToMs('not-a-time')).toBeNull()
  })
})

describe('toInvestigationFilter', () => {
  it('carries the resolved bracket plus the echoes', () => {
    const draft = {
      ...emptyDraft(),
      fromMs: NOW - HOUR,
      toMs: NOW,
      windowMinutes: null,
      domain: 'nacos',
      instance: 'dns-node-2',
    }
    expect(toInvestigationFilter(draft, NOW)).toEqual({
      from: NOW - HOUR,
      to: NOW,
      domain: 'nacos',
      responseCode: null,
      instance: 'dns-node-2',
    })
  })
})
