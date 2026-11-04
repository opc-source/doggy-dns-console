import { createMemoryHistory, createRouter } from 'vue-router'
import { describe, expect, it } from 'vitest'

import { routes } from '@/router'

describe('router', () => {
  it('nests the detail route under the failure list', () => {
    const list = routes.find((route) => route.name === 'failures-list')
    // `routes` is the app's own union of record shapes, so the children only
    // become visible once narrowed, and optional once narrowed to.
    const children = list && 'children' in list ? (list.children ?? []) : []

    expect(list?.path).toBe('/failures')
    expect(children.map((child) => child.path)).toContain(':traceId')

    const detail = children.find((child) => child.name === 'failure-detail')
    expect(detail?.path).toBe(':traceId')
    expect(detail?.name).toBe('failure-detail')
  })

  it('keeps the bracket in the detail link', () => {
    const router = createRouter({ history: createMemoryHistory(), routes })

    const detail = router.resolve({
      name: 'failure-detail',
      params: { traceId: 't1' },
      query: { windowMinutes: '10080' },
    })

    expect(detail.path).toBe('/failures/t1')
    expect(detail.query.windowMinutes).toBe('10080')
  })

  it('resolves the detail deep link with its analysis query', async () => {
    const router = createRouter({ history: createMemoryHistory(), routes })

    await router.push('/failures/t1?from=100&to=200&analysis=a1')

    const route = router.currentRoute.value
    expect(route.name).toBe('failure-detail')
    expect(route.params.traceId).toBe('t1')
    expect(route.query.from).toBe('100')
    expect(route.query.analysis).toBe('a1')
  })
})
