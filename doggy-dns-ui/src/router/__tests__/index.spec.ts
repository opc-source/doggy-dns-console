import { createMemoryHistory, createRouter } from 'vue-router'
import { describe, expect, it } from 'vitest'

import { routes } from '@/router'

describe('router', () => {
  it('resolves the failure deep link with its window and analysis query', async () => {
    const router = createRouter({ history: createMemoryHistory(), routes })

    await router.push('/failures/t1?windowMinutes=360&analysis=a1')

    const route = router.currentRoute.value
    expect(route.name).toBe('failure')
    expect(route.params.traceId).toBe('t1')
    expect(route.query.windowMinutes).toBe('360')
    expect(route.query.analysis).toBe('a1')
  })
})
