import { mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent } from 'vue'
import { createMemoryHistory, createRouter } from 'vue-router'
import { afterEach, describe, expect, it } from 'vitest'

import FailureList from '../FailureList.vue'
import type { AnalysisMarker, FailureGroup } from '@/api/types'
import { msToDatetime } from '@/lib/failureFilter'

const LONG_QUERY = 'nacos-a.internal.nsvc.svc.cluster.local.test.'

const GROUP: FailureGroup = {
  key: 'trace:4bf9',
  traceId: '4bf92f3577b34da6',
  query: LONG_QUERY,
  qtype: 'A',
  instance: 'dns-node-2',
  authority: 'forward[8.8.8.8:53]',
  responseCode: 'SERVFAIL',
  error: 'upstream timed out',
  count: 12,
  firstSeen: 1_761_433_200_000,
  lastSeen: 1_761_436_800_000,
  durationMs: 5012,
}

/** A grouped row with no trace, host or duration: everything nullable. */
const UNLABELED: FailureGroup = {
  key: 'query:b.example.',
  traceId: null,
  query: 'b.example.',
  qtype: 'AAAA',
  instance: null,
  authority: null,
  responseCode: 'REFUSED',
  error: null,
  count: 3,
  firstSeen: 1_761_433_200_000,
  lastSeen: 1_761_436_800_000,
  durationMs: null,
}

function markerFor(
  traceId: string,
  status: AnalysisMarker['status'],
  overrides: Partial<AnalysisMarker> = {},
): AnalysisMarker {
  return {
    id: `a-${traceId}`,
    traceId,
    status,
    verdictCategory: status === 'succeeded' ? 'upstream_timeout' : null,
    confidence: status === 'succeeded' ? 'high' : null,
    attempt: 2,
    createdAt: 1_761_433_200_000,
    finishedAt: status === 'succeeded' ? 1_761_433_260_000 : null,
    ...overrides,
  }
}

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', name: 'overview', component: defineComponent({ name: 'RootStub' }) },
      {
        path: '/failures',
        name: 'failures-list',
        component: defineComponent({ name: 'ListStub' }),
      },
    ],
  })
}

const wrappers: VueWrapper[] = []

function mountList(
  props: Partial<{
    groups: FailureGroup[]
    markers: Map<string, AnalysisMarker>
    limit: number
    seeAll: boolean
    dense: boolean
    title: string
    subtitle: string
    emptyText: string
  }> = {},
) {
  const router = makeRouter()
  const wrapper = mount(FailureList, {
    props: { groups: [GROUP], ...props },
    global: { plugins: [router] },
  })
  wrappers.push(wrapper)
  return { router, wrapper }
}

afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
})

describe('FailureList', () => {
  it('keeps the dense card row down to code, qtype, query, count and badge', () => {
    const { wrapper } = mountList({
      groups: [{ ...GROUP, qtype: 'MX' }],
      markers: new Map([[GROUP.traceId as string, markerFor(GROUP.traceId as string, 'succeeded')]]),
    })

    const row = wrapper.find('[data-testid="failure-row"]')
    expect(row.text()).toContain('SERVFAIL')
    // Seven fields means qtype too: the compact card row shows it as well.
    expect(row.text()).toContain('MX')
    expect(row.text()).toContain(LONG_QUERY.slice(0, 39))
    expect(row.text()).toContain('…')
    expect(row.text()).not.toContain(LONG_QUERY)
    expect(row.text()).toContain(String(GROUP.count))
    expect(row.find('[data-testid="analyzed-badge"]').text()).toContain('已有分析')
    // The overview card has no room for a host column, and its e2e case is the
    // page's, so the dense row must not claim the test id.
    expect(row.find('[data-testid="row-instance"]').exists()).toBe(false)
    expect(row.text()).not.toContain(GROUP.authority as string)
  })

  it('names the verdict on a finished analysis', () => {
    const { wrapper } = mountList({
      markers: new Map([
        [
          GROUP.traceId as string,
          markerFor(GROUP.traceId as string, 'succeeded', { verdictCategory: 'nacos_stale' }),
        ],
      ]),
    })

    expect(wrapper.get('[data-testid="analyzed-badge"]').text()).toBe('已有分析 · nacos_stale')
  })

  it.each([
    ['queued', '分析中'],
    ['running', '分析中'],
    ['failed', '已有分析 · 失败'],
    ['invalid', '已有分析 · 结果非法'],
  ] as const)('labels a %s attempt as %s', (status, expected) => {
    const { wrapper } = mountList({
      markers: new Map([[GROUP.traceId as string, markerFor(GROUP.traceId as string, status)]]),
    })

    expect(wrapper.get('[data-testid="analyzed-badge"]').text()).toBe(expected)
  })

  it('names the attempt the badge points at', () => {
    const { wrapper } = mountList({
      markers: new Map([
        [
          GROUP.traceId as string,
          markerFor(GROUP.traceId as string, 'invalid', { id: 'analysis-77', attempt: 4 }),
        ],
      ]),
    })

    const tooltip = wrapper.get('[data-testid="analyzed-badge"]').attributes('title') ?? ''
    expect(tooltip).toContain('analysis-77')
    expect(tooltip).toContain('第 4 次')
  })

  it('shows no badge for a trace the marker answer left out', () => {
    const { wrapper } = mountList({ markers: new Map([['other', markerFor('other', 'succeeded')]]) })

    expect(wrapper.find('[data-testid="analyzed-badge"]').exists()).toBe(false)
  })

  it('adds the host, authority and timestamps in full mode', () => {
    const { wrapper } = mountList({ dense: false, groups: [{ ...GROUP, qtype: 'MX' }] })

    const row = wrapper.find('[data-testid="failure-row"]')
    expect(row.text()).toContain('MX')
    expect(row.find('[data-testid="row-instance"]').text()).toBe('dns-node-2')
    expect(row.text()).toContain('forward[8.8.8.8:53]')
    expect(row.text()).toContain(msToDatetime(GROUP.firstSeen))
    expect(row.text()).toContain(msToDatetime(GROUP.lastSeen))
    expect(row.text()).toContain('upstream timed out')
    expect(row.text()).toContain('5012')
    // The page row has room to wrap, so the query never arrives cut.
    expect(row.text()).toContain(LONG_QUERY)
  })

  it("moves the page row's meta fields off the query's line", () => {
    // Browser-measured at a 1440 x 900 viewport: the page row put nine `shrink-0`
    // spans plus the query in one flex line, and they needed ~1014px of a 874px
    // content box. The query was the only shrinkable child, so it went to 0 width
    // and `break-all` painted it one character per line (a 656px-tall row). The
    // meta fields now own a second line, which jsdom can see as a second parent.
    const { wrapper } = mountList({ dense: false, groups: [GROUP] })
    const row = wrapper.get('[data-testid="failure-row"]')
    const query = row.get('[data-testid="row-query"]')
    const meta = row.get('[data-testid="row-meta"]')

    expect(query.element.parentElement).not.toBe(meta.element.parentElement)
    expect(meta.text()).toContain('dns-node-2')
    expect(meta.text()).toContain(msToDatetime(GROUP.firstSeen))
    expect(meta.text()).toContain('5012')
    // The dense card row has no meta line, so it keeps its single line.
    expect(mountList({ groups: [GROUP] }).wrapper.find('[data-testid="row-meta"]').exists()).toBe(false)
  })

  it('truncates a dense query only past 42 characters', () => {
    // Distinct characters around the cut: a longer prefix must not hide inside the
    // expected one, and a shorter one must not match it either.
    const whole = `a${'b'.repeat(41)}`
    const over = `${whole}c`

    const atTheLimit = mountList({ groups: [{ ...GROUP, key: 'trace:42', query: whole }] })
      .wrapper.find('[data-testid="failure-row"]')
      .text()
    expect(atTheLimit).toContain(whole)
    expect(atTheLimit).not.toContain('…')

    const justOver = mountList({ groups: [{ ...GROUP, key: 'trace:43', query: over }] })
      .wrapper.find('[data-testid="failure-row"]')
      .text()
    expect(justOver).toContain(`${over.slice(0, 39)}…`)
    expect(justOver).not.toContain(over)
  })

  it('keeps the card empty state in English and lets the page replace it', () => {
    expect(mountList({ groups: [] }).wrapper.text()).toContain('No failures in this window.')

    const page = mountList({ groups: [], emptyText: '当前区间没有失败记录。' }).wrapper.text()
    expect(page).toContain('当前区间没有失败记录。')
    expect(page).not.toContain('No failures in this window.')
  })

  it('renders a missing field as a dash, never as null', () => {
    const { wrapper } = mountList({ dense: false, groups: [UNLABELED] })

    const row = wrapper.find('[data-testid="failure-row"]')
    expect(row.text()).not.toContain('null')
    expect(row.find('[data-testid="row-instance"]').text()).toBe('—')
  })

  it('renders an omitted wire field as a dash, never as undefined', () => {
    // The wire may omit a grouped field outright, which the `FailureGroup`
    // interface does not model: the helper still has to render a label, not the
    // string "undefined".
    const omitted = { ...UNLABELED, authority: undefined } as unknown as FailureGroup
    const { wrapper } = mountList({ dense: false, groups: [omitted] })

    const row = wrapper.find('[data-testid="failure-row"]')
    expect(row.text()).not.toContain('undefined')
    expect(row.text()).not.toContain('null')
  })

  it('emits the clicked group so the parent can build the detail link', async () => {
    const { wrapper } = mountList()

    await wrapper.find('[data-testid="failure-row"]').trigger('click')

    expect(wrapper.emitted('select')?.[0]).toEqual([GROUP])
  })

  it('keeps the overview card at five rows and lets the page ask for more', () => {
    const groups = Array.from({ length: 8 }, (_unused, index) => ({
      ...GROUP,
      key: `trace:t${index}`,
      traceId: `t${index}`,
    }))

    expect(mountList({ groups }).wrapper.findAll('[data-testid="failure-row"]')).toHaveLength(5)
    expect(
      mountList({ groups, limit: 200 }).wrapper.findAll('[data-testid="failure-row"]'),
    ).toHaveLength(8)
  })

  it('links to the whole list only when the card is not the list', () => {
    const card = mountList({ title: 'Top failures', subtitle: 'last window' })
    expect(card.wrapper.find('[data-testid="see-all"]').exists()).toBe(false)

    const { wrapper, router } = mountList({ seeAll: true })
    const link = wrapper.find('[data-testid="see-all"]')
    expect(link.text()).toBe('查看全部')
    expect(link.attributes('href')).toBe(router.resolve({ name: 'failures-list' }).href)
  })
})
