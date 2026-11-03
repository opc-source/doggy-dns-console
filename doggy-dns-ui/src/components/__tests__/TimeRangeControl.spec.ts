import { mount, type VueWrapper } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import TimeRangeControl from '@/components/TimeRangeControl.vue'
import {
  DEFAULT_WINDOW_MINUTES,
  emptyDraft,
  msToDatetime,
  type FilterDraft,
} from '@/lib/failureFilter'

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

// Drive a `datetime-local` input the way the operator does: set its DOM value and
// fire the `change` event these inputs actually bind (`@change` in
// `TimeRangeControl.vue:118,128`, no `v-model`). `setValue` would reach the same
// handler — Vue Test Utils dispatches `change` after `input` — but naming the
// bound event keeps the contract visible in the helper.
async function typeDatetime(wrapper: VueWrapper, testId: string, value: string) {
  const input = wrapper.get(`[data-testid="${testId}"]`)
  ;(input.element as HTMLInputElement).value = value
  await input.trigger('change')
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

  // (a) `setTo` backwards completion. These three cases are the control's OWN
  // contract, previously guarded only through `FailureFilterBar.spec.ts`'s
  // positional `dateInput()` helper, so the component could regress with nothing
  // going red. Each drives the real selector and asserts the emitted draft.
  it('completes a 止-only edit backwards from the typed end, not from now', async () => {
    const wrapper = mount(TimeRangeControl, { props: { modelValue: emptyDraft() } })
    // An end two hours ago; a start derived from `Date.now()` would land AFTER it
    // and the control would hand the page a bracket it refuses. Backwards
    // completion keeps the start one default window before the typed end.
    const toText = msToDatetime(Date.now() - 2 * HOUR)

    await typeDatetime(wrapper, 'range-to', toText)

    const toMs = Date.parse(toText)
    expect(lastDraft(wrapper)).toEqual({
      ...emptyDraft(),
      toMs,
      fromMs: toMs - DEFAULT_WINDOW_MINUTES * 60_000,
      windowMinutes: null,
    })
    // The bracket the operator can actually submit: start strictly before end.
    const emitted = lastDraft(wrapper)
    expect(emitted.fromMs!).toBeLessThan(emitted.toMs!)
  })

  // (b) cleared input falls back to the default relative window, both sides.
  it('drops a cleared 起 back to the default relative window', async () => {
    const wrapper = mount(TimeRangeControl, {
      props: { modelValue: { ...emptyDraft(), fromMs: Date.now() - 3 * HOUR, toMs: Date.now() } },
    })

    await typeDatetime(wrapper, 'range-from', '')

    expect(lastDraft(wrapper)).toEqual({
      ...emptyDraft(),
      fromMs: null,
      toMs: null,
      windowMinutes: DEFAULT_WINDOW_MINUTES,
    })
  })

  it('drops a cleared 止 back to the default relative window', async () => {
    const wrapper = mount(TimeRangeControl, {
      props: { modelValue: { ...emptyDraft(), fromMs: Date.now() - 3 * HOUR, toMs: Date.now() } },
    })

    await typeDatetime(wrapper, 'range-to', '')

    expect(lastDraft(wrapper)).toEqual({
      ...emptyDraft(),
      fromMs: null,
      toMs: null,
      windowMinutes: DEFAULT_WINDOW_MINUTES,
    })
  })

  // (c) a preset click rebuilds the time fields but preserves the non-time ones.
  it('keeps domain, responseCode and instance when a preset rebuilds the window', async () => {
    const wrapper = mount(TimeRangeControl, {
      props: {
        modelValue: {
          ...emptyDraft(),
          fromMs: Date.now() - 3 * HOUR,
          toMs: Date.now(),
          domain: 'nacos.local',
          responseCode: 'SERVFAIL',
          instance: 'dns-1',
        },
      },
    })

    await wrapper.get('[data-testid="quick-6h"]').trigger('click')

    // Clicking 6 小时 must swap the absolute pair for a relative window without
    // wiping the filters the operator already set.
    expect(lastDraft(wrapper)).toEqual({
      ...emptyDraft(),
      fromMs: null,
      toMs: null,
      windowMinutes: 360,
      domain: 'nacos.local',
      responseCode: 'SERVFAIL',
      instance: 'dns-1',
    })
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
