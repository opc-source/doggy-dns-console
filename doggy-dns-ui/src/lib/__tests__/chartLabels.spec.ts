import { describe, expect, it } from 'vitest'

import { seriesSpanMs, seriesTick } from '@/lib/chartLabels'

const DAY = 24 * 60 * 60 * 1000
const HOUR = 60 * 60 * 1000
// 本地时区构造,避开 `Date.parse` 的时区/夏令时归一化。
const TS = new Date(2026, 0, 1, 3, 0).getTime()
const hourOnly = new Date(TS).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
const dated = new Date(TS).toLocaleDateString([], { month: '2-digit', day: '2-digit' })

describe('seriesTick', () => {
  it('keeps the hour-only label inside a day', () => {
    expect(seriesTick(TS, HOUR)).toBe(hourOnly)
    expect(seriesTick(TS, DAY)).toBe(hourOnly)
  })

  it('prefixes the date once the span passes a day', () => {
    expect(seriesTick(TS, DAY + 1)).toBe(`${dated} ${hourOnly}`)
    expect(seriesTick(TS, 7 * DAY)).toBe(`${dated} ${hourOnly}`)
  })

  it('has no span for fewer than two points', () => {
    expect(seriesSpanMs([])).toBe(0)
    expect(seriesSpanMs([{ ts: TS }])).toBe(0)
    expect(seriesSpanMs([{ ts: TS }, { ts: TS + 1000 }])).toBe(1000)
  })
})
