import { mount, type VueWrapper } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import TimeRangeControl from '@/components/TimeRangeControl.vue'
import { emptyDraft, msToDatetime, type FilterDraft } from '@/lib/failureFilter'

const HOUR = 3_600_000

// `VueWrapper`, the same handle `FailureFilterBar.spec.ts` uses: `mount` is
// generic, so `ReturnType<typeof mount>` collapses to a wrapper the helpers
// cannot index.
function hintText(wrapper: VueWrapper): string {
  return wrapper.get('[data-testid="range-hint"]').text()
}

function lastDraft(wrapper: VueWrapper): FilterDraft {
  const events = wrapper.emitted('update:modelValue')
  if (!events || events.length === 0) throw new Error('the control emitted no draft')
  return events[events.length - 1]![0] as FilterDraft
}

describe('TimeRangeControl', () => {
  it('offers 1h, 6h, 24h and 7d presets and marks the active one', async () => {
    const wrapper = mount(TimeRangeControl, {
      props: { modelValue: { ...emptyDraft(), windowMinutes: 360 } },
    })
    for (const testId of ['quick-1h', 'quick-6h', 'quick-24h', 'quick-7d']) {
      expect(wrapper.find(`[data-testid="${testId}"]`).exists()).toBe(true)
    }
    expect(wrapper.get('[data-testid="quick-6h"]').attributes('aria-pressed')).toBe('true')
    await wrapper.get('[data-testid="quick-7d"]').trigger('click')
    expect(lastDraft(wrapper)).toMatchObject({ fromMs: null, toMs: null, windowMinutes: 10080 })
  })

  it('never emits a half bracket', async () => {
    const wrapper = mount(TimeRangeControl, { props: { modelValue: emptyDraft() } })
    // An hour ago, not a fixed 2026-01-01: `setFrom` completes the pair with the
    // browser's now, so a dated-from would make the emitted bracket wider than
    // the very ceiling the hint below advertises — the test would pass while
    // proving nothing about a draft the page could actually submit.
    const fromText = msToDatetime(Date.now() - HOUR)

    await wrapper.get('[data-testid="range-from"]').setValue(fromText)

    const draft = lastDraft(wrapper)
    expect(draft.fromMs).toBe(Date.parse(fromText))
    expect(draft.toMs).not.toBeNull()
    expect(draft.windowMinutes).toBeNull()
  })

  it('states the one ceiling and the one endpoint without a window', () => {
    const hint = hintText(mount(TimeRangeControl, { props: { modelValue: emptyDraft() } }))
    expect(hint).toContain('5 分钟到 10080 分钟')
    // The old copy ("看板仍限 1440 分钟") is now false; the new copy must not
    // re-grow a per-endpoint number.
    expect(hint).not.toContain('1440')
    expect(hint).toContain('/api/instances')
  })
})
