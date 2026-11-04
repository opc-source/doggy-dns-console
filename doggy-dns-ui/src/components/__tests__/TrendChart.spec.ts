import { shallowMount, type VueWrapper } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import TrendChart from '@/components/TrendChart.vue'
import VChart from 'vue-echarts'
import type { SeriesPoint } from '@/api/types'
import { seriesSpanMs, seriesTick } from '@/lib/chartLabels'

const DAY = 24 * 60 * 60 * 1000
// Local-time construction, as in `chartLabels.spec.ts`: it sidesteps
// `Date.parse`'s timezone/DST normalisation so the expected labels are stable.
const T0 = new Date(2026, 0, 1, 3, 0).getTime()

function xAxisData(wrapper: VueWrapper): string[] {
  // `shallowMount` replaces `VChart` with a stub, so echarts never boots (no
  // canvas in jsdom) while the stub still receives the `option` prop. Reading it
  // asserts what the chart is actually handed rather than recomputing it here.
  return (wrapper.findComponent(VChart).props('option') as { xAxis: { data: string[] } }).xAxis
    .data
}


function expectLabels(series: SeriesPoint[], labels: string[]) {
  const spanMs = seriesSpanMs(series)
  // 1:1 alignment with the series and the date-prefixed shape `seriesTick` gives
  // for a span past a day. If `TrendChart` reverted to an inline
  // `toLocaleTimeString` map the hour-only labels would neither match this nor
  // carry the date prefix.
  expect(labels).toHaveLength(series.length)
  expect(labels).toEqual(series.map((point) => seriesTick(point.ts, spanMs)))
  // Locale-agnostic proof of the date prefix: a >1-day span labels each point
  // with its `toLocaleDateString` day, so a label is longer than the bare hour.
  const hourOnly = new Date(series[0]!.ts).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  })
  expect(labels[0]).not.toBe(hourOnly)
  expect(labels[0]!.length).toBeGreaterThan(hourOnly.length)
}

describe('TrendChart', () => {
  it('feeds date-prefixed, series-aligned tick labels to the chart', () => {
    const series: SeriesPoint[] = [
      { ts: T0, queries: 10, failures: 1 },
      { ts: T0 + 2 * DAY, queries: 20, failures: 2 },
    ]
    const wrapper = shallowMount(TrendChart, {
      props: { series },
    })

    // Direct assertion here too: the stub is what the component hands the chart,
    // so its very existence is part of the wiring being pinned.
    expect(wrapper.findComponent(VChart).exists()).toBe(true)
    expectLabels(series, xAxisData(wrapper))
  })

  it('recomputes the labels when the series prop is replaced', async () => {
    const first: SeriesPoint[] = [
      { ts: T0, queries: 10, failures: 1 },
      { ts: T0 + 2 * DAY, queries: 20, failures: 2 },
    ]
    const wrapper = shallowMount(TrendChart, {
      props: { series: first },
    })
    const before = xAxisData(wrapper)

    const second: SeriesPoint[] = [
      { ts: T0 + 5 * DAY, queries: 5, failures: 0 },
      { ts: T0 + 7 * DAY, queries: 9, failures: 1 },
    ]
    await wrapper.setProps({ series: second })
    const after = xAxisData(wrapper)

    expectLabels(second, after)
    // A different pair of days: the labels tracked the new `series`, they were
    // not frozen at mount.
    expect(after).not.toEqual(before)
  })
})
