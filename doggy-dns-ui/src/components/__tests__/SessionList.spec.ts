import { mount } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import SessionList from '@/components/SessionList.vue'
import type { SessionSummary } from '@/api/types'

const wrappers: ReturnType<typeof mount>[] = []

function session(over: Partial<SessionSummary> = {}): SessionSummary {
  return {
    id: 's-0f3a91c2-77bd-4a18-9b0c-1',
    appName: 'doggy-dns-console',
    userId: 'local',
    lastUpdateTime: 1_761_436_800,
    events: [],
    state: {},
    ...over,
  }
}

function mountList(sessions: SessionSummary[]) {
  const wrapper = mount(SessionList, { props: { sessions, selected: null } })
  wrappers.push(wrapper)
  return wrapper
}

afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
})

describe('SessionList', () => {
  it('titles a session by the state it was created with', () => {
    const wrapper = mountList([
      session({ state: { title: '追问 a.example. · upstream_timeout', traceId: 't1' } }),
    ])

    expect(wrapper.find('[data-test="session-title"]').text()).toBe(
      '追问 a.example. · upstream_timeout',
    )
  })

  it('keeps the whole label reachable when the 300px column clips it', () => {
    const wrapper = mountList([
      session({ state: { title: '追问 a.example. · upstream_timeout', traceId: 't1' } }),
    ])

    // The row truncates, so hover is the only way to read a long title.
    expect(wrapper.get('button[title]').attributes('title')).toBe(
      '追问 a.example. · upstream_timeout',
    )
  })

  it('names an unlabelled session by its id instead of digging through events', () => {
    // The list endpoint never returns events, so reading them could only ever
    // yield the placeholder — and an old session that does carry them must not
    // be titled from a message the operator cannot see in the list.
    const wrapper = mountList([
      session({
        state: {},
        events: [{ author: 'user', content: { role: 'user', parts: [{ text: '隐藏的旧内容' }] } }],
      }),
    ])

    const title = wrapper.find('[data-test="session-title"]').text()
    expect(title).toBe('（无标题）s-0f3a91')
    expect(title).not.toContain('隐藏的旧内容')
  })
})
