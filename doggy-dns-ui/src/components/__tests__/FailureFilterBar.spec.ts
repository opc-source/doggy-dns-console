import { mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import FailureFilterBar from '../FailureFilterBar.vue'
import { emptyDraft, msToDatetime, relativeDraft, type FilterDraft } from '@/lib/failureFilter'

const HOUR = 3_600_000
const DAY = 24 * HOUR
const NOW = 1_761_436_800_000

const wrappers: VueWrapper[] = []

function mountBar(modelValue: FilterDraft): VueWrapper {
  const wrapper = mount(FailureFilterBar, { props: { modelValue, instances: [] } })
  wrappers.push(wrapper)
  return wrapper
}

/** The bar's two date inputs, in template order: 起 then 止. */
function dateInput(wrapper: VueWrapper, index: number) {
  const input = wrapper.findAll('input[type="datetime-local"]')[index]
  if (input === undefined) throw new Error('the filter bar has no date input')
  return input
}

function lastEmit(wrapper: VueWrapper): FilterDraft {
  const events = wrapper.emitted('update:modelValue')
  if (!events || events.length === 0) throw new Error('the bar emitted no draft')
  return events[events.length - 1]![0] as FilterDraft
}

afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
})

describe('FailureFilterBar', () => {
  it('completes a 止-only edit backwards from the typed end, not from now', async () => {
    const toText = msToDatetime(NOW - 3 * DAY)
    const wrapper = mountBar(emptyDraft())

    await dateInput(wrapper, 1).setValue(toText)

    // The typed end is three days old; a start derived from `Date.now()` would
    // land after it, so the page would refuse its own input.
    const toMs = Date.parse(toText)
    expect(lastEmit(wrapper)).toEqual({
      ...emptyDraft(),
      toMs,
      fromMs: toMs - HOUR,
      windowMinutes: null,
    })
  })

  it('completes a 起-only edit with an end at the bar own now', async () => {
    const before = Date.now()
    const fromText = msToDatetime(before - 2 * HOUR)
    const wrapper = mountBar(emptyDraft())

    await dateInput(wrapper, 0).setValue(fromText)

    const emitted = lastEmit(wrapper)
    expect(emitted.fromMs).toBe(Date.parse(fromText))
    expect(emitted.windowMinutes).toBeNull()
    expect(emitted.toMs).not.toBeNull()
    expect(emitted.toMs!).toBeGreaterThanOrEqual(before)
    expect(emitted.toMs!).toBeLessThanOrEqual(Date.now())
  })

  it('drops an absolute pair for a quick window and keeps the other filters', async () => {
    const wrapper = mountBar({
      ...emptyDraft(),
      fromMs: NOW - HOUR,
      toMs: NOW,
      domain: 'nacos.local',
      responseCode: 'SERVFAIL',
      instance: 'dns-1',
    })

    await wrapper.find('[data-testid="quick-7d"]').trigger('click')

    expect(lastEmit(wrapper)).toEqual({
      ...emptyDraft(),
      fromMs: null,
      toMs: null,
      windowMinutes: 10080,
      domain: 'nacos.local',
      responseCode: 'SERVFAIL',
      instance: 'dns-1',
    })
  })

  it('falls back to the default window when a date input is cleared', async () => {
    const draft = { ...emptyDraft(), fromMs: NOW - 3 * HOUR, toMs: NOW }

    const clearedStart = mountBar(draft)
    await dateInput(clearedStart, 0).setValue('')
    expect(lastEmit(clearedStart)).toEqual({
      ...emptyDraft(),
      fromMs: null,
      toMs: null,
      windowMinutes: 60,
    })

    const clearedEnd = mountBar(draft)
    await dateInput(clearedEnd, 1).setValue('')
    expect(lastEmit(clearedEnd)).toEqual({
      ...emptyDraft(),
      fromMs: null,
      toMs: null,
      windowMinutes: 60,
    })
  })

  it('marks only the active quick window as pressed', () => {
    const relative = mountBar(relativeDraft(60))
    expect(relative.find('[data-testid="quick-1h"]').attributes('aria-pressed')).toBe('true')
    expect(relative.find('[data-testid="quick-24h"]').attributes('aria-pressed')).toBe('false')
    expect(relative.find('[data-testid="quick-7d"]').attributes('aria-pressed')).toBe('false')

    // An absolute pair is not any chip: none of them may claim it.
    const absolute = mountBar({ ...emptyDraft(), fromMs: NOW - 3 * HOUR, toMs: NOW })
    for (const id of ['quick-1h', 'quick-24h', 'quick-7d']) {
      expect(absolute.find(`[data-testid="${id}"]`).attributes('aria-pressed')).toBe('false')
    }
  })
})
