import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useChatStore } from '@/stores/chat'

describe('chat store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('reports a session-creation failure without losing the draft state', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response(JSON.stringify({ error: 'boom' }), { status: 500 })),
    )
    const chat = useChatStore()
    const sent = await chat.send('你好')
    expect(sent).toBe(false)
    expect(chat.messages).toHaveLength(0)
    expect(chat.error).toBeTruthy()
  })
})
