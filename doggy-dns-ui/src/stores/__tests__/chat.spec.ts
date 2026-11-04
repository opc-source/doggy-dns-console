import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { SessionEvent } from '@/api/types'
import { useChatStore } from '@/stores/chat'
import type { ChatMessage } from '@/lib/chatModel'

function sse(value: unknown) {
  return `data: ${JSON.stringify(value)}\n\n`
}

function jsonFetch(payload: unknown) {
  return new Response(JSON.stringify(payload), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  })
}

function streamingResponse(chunks: string[]) {
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(new TextEncoder().encode(chunk))
      controller.close()
    },
  })
  return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } })
}

/**
 * A stream that stays open until the request is aborted, the way a live run does.
 * A retired reader has to reject before the store can close out its message.
 */
function openStreamUntilAborted(chunks: string[], signal?: AbortSignal) {
  let controller!: ReadableStreamDefaultController<Uint8Array>
  const body = new ReadableStream<Uint8Array>({
    start(created) {
      controller = created
      for (const chunk of chunks) created.enqueue(new TextEncoder().encode(chunk))
    },
  })
  signal?.addEventListener('abort', () =>
    controller.error(new DOMException('Aborted', 'AbortError')),
  )
  return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } })
}

const TOOL_STARTED = sse({ type: 'TOOL_CALL_START', toolCallId: 'c1', toolCallName: 'get_overview' })

function toolStatus(chat: ReturnType<typeof useChatStore>, messageIndex: number) {
  const tool = chat.messages[messageIndex]?.segments[0]
  return tool?.kind === 'tool' ? tool.status : undefined
}

function kinds(messages: ChatMessage[], index: number) {
  return (messages[index]?.segments ?? []).map((segment) => segment.kind)
}

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

  it('streams a run into one assistant message keeping thinking, text and tool order', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (path: string) => {
        if (path.endsWith('/sessions')) return jsonFetch({ id: 's1' })
        return streamingResponse([
          sse({ type: 'REASONING_MESSAGE_CONTENT', messageId: 'r1', delta: '先看 KPI。' }),
          sse({ type: 'TOOL_CALL_START', toolCallId: 'c1', toolCallName: 'get_overview' }),
          sse({ type: 'TOOL_CALL_ARGS', toolCallId: 'c1', delta: '{"windowMinutes":60}' }),
          sse({ type: 'TOOL_CALL_RESULT', toolCallId: 'c1', content: '{"queries":128934}' }),
          sse({ type: 'TEXT_MESSAGE_CONTENT', messageId: 'm1', delta: '成功率 99.7%。' }),
          sse({ type: 'RUN_FINISHED' }),
        ])
      }),
    )
    const chat = useChatStore()
    expect(await chat.send('整体健康度如何')).toBe(true)

    expect(chat.messages.map((message) => message.role)).toEqual(['user', 'assistant'])
    expect(kinds(chat.messages, 0)).toEqual(['text'])
    expect(kinds(chat.messages, 1)).toEqual(['thinking', 'tool', 'text'])

    const [thinking, tool, text] = chat.messages[1]?.segments ?? []
    expect(thinking?.kind === 'thinking' && thinking.text).toBe('先看 KPI。')
    expect(tool?.kind === 'tool' && tool.name).toBe('get_overview')
    expect(tool?.kind === 'tool' && tool.args).toBe('{"windowMinutes":60}')
    expect(tool?.kind === 'tool' && tool.result).toBe('{"queries":128934}')
    expect(tool?.kind === 'tool' && tool.status).toBe('done')
    expect(text?.kind === 'text' && text.text).toBe('成功率 99.7%。')
  })

  it('keeps the assistant message visible while the run is still streaming', async () => {
    let releaseStream!: () => void
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode(sse({
          type: 'TEXT_MESSAGE_CONTENT',
          messageId: 'm1',
          delta: '前半段',
        })))
        releaseStream = () => {
          controller.enqueue(
            new TextEncoder().encode(sse({ type: 'RUN_FINISHED' })),
          )
          controller.close()
        }
      },
    })
    vi.stubGlobal(
      'fetch',
      vi.fn(async (path: string) => {
        if (path.endsWith('/sessions')) return jsonFetch({ id: 's1' })
        return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } })
      }),
    )
    const chat = useChatStore()
    const pending = chat.send('继续')
    await vi.waitFor(() => expect(chat.messages.length).toBe(2))

    const assistant = chat.messages[1]!
    expect(assistant.segments[0]?.kind === 'text' && assistant.segments[0].text).toBe('前半段')

    releaseStream()
    await pending
    expect(chat.running).toBe(false)
    expect(assistant.segments[0]?.kind === 'text' && assistant.segments[0].text).toBe('前半段')
  })

  it('stops the in-flight run when the transcript is cleared', async () => {
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode(sse({
          type: 'TEXT_MESSAGE_CONTENT',
          messageId: 'm1',
          delta: '还在写',
        })))
      },
    })
    vi.stubGlobal(
      'fetch',
      vi.fn(async (path: string) => {
        if (path.endsWith('/sessions')) return jsonFetch({ id: 's1' })
        return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } })
      }),
    )
    const chat = useChatStore()
    const pending = chat.send('继续')
    await vi.waitFor(() => expect(chat.running).toBe(true))

    // SessionsView starts a new chat this way; the old run must not outlive it.
    chat.reset()
    expect(chat.running).toBe(false)

    await pending
    expect(chat.messages).toHaveLength(0)
  })

  it('settles a tool call the stopped run never answered', async () => {
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(
          new TextEncoder().encode(
            sse({ type: 'TOOL_CALL_START', toolCallId: 'c1', toolCallName: 'get_overview' }),
          ),
        )
      },
    })
    vi.stubGlobal(
      'fetch',
      vi.fn(async (path: string) => {
        if (path.endsWith('/sessions')) return jsonFetch({ id: 's1' })
        return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } })
      }),
    )
    const chat = useChatStore()
    const pending = chat.send('查一下')
    await vi.waitFor(() => expect(chat.messages[1]?.segments.length).toBe(1))

    chat.stop()
    await pending

    const tool = chat.messages[1]?.segments[0]
    expect(tool?.kind === 'tool' && tool.status).toBe('error')
  })

  it('settles the tool card a finished run never answered', async () => {
    // A held or abandoned call gets no TOOL_CALL_RESULT in this run at all.
    vi.stubGlobal(
      'fetch',
      vi.fn(async (path: string) => {
        if (path.endsWith('/sessions')) return jsonFetch({ id: 's1' })
        return streamingResponse([TOOL_STARTED, sse({ type: 'RUN_FINISHED' })])
      }),
    )
    const chat = useChatStore()
    await chat.send('查一下')

    expect(toolStatus(chat, 1)).toBe('error')
  })

  it('settles the tool card when the stream closes before the run finishes', async () => {
    // A restarting console drops the response body without RUN_FINISHED or RUN_ERROR.
    vi.stubGlobal(
      'fetch',
      vi.fn(async (path: string) => {
        if (path.endsWith('/sessions')) return jsonFetch({ id: 's1' })
        return streamingResponse([TOOL_STARTED])
      }),
    )
    const chat = useChatStore()
    await chat.send('查一下')

    expect(toolStatus(chat, 1)).toBe('error')
  })

  it('settles the tool card of the run a new question superseded', async () => {
    let runs = 0
    vi.stubGlobal(
      'fetch',
      vi.fn(async (path: string, init?: { signal?: AbortSignal }) => {
        if (path.endsWith('/sessions')) return jsonFetch({ id: 's1' })
        runs += 1
        return runs === 1
          ? openStreamUntilAborted([TOOL_STARTED], init?.signal)
          : streamingResponse([
              sse({ type: 'TEXT_MESSAGE_CONTENT', messageId: 'm2', delta: '新的回答。' }),
              sse({ type: 'RUN_FINISHED' }),
            ])
      }),
    )
    const chat = useChatStore()
    const firstRun = chat.send('查一下')
    await vi.waitFor(() => expect(chat.messages[1]?.segments.length).toBe(1))

    // 继续追问 while the first answer is still streaming.
    await chat.send('再查一次')
    await firstRun

    expect(toolStatus(chat, 1)).toBe('error')
    expect(chat.running).toBe(false)
    expect(chat.messages[3]?.segments[0]?.kind === 'text' && chat.messages[3]?.segments[0].text).toBe(
      '新的回答。',
    )
  })

  it('creates the session with the labels it was handed', async () => {
    const bodies: unknown[] = []
    vi.stubGlobal(
      'fetch',
      vi.fn(async (path: string, init?: { body?: string }) => {
        if (path.endsWith('/sessions')) {
          bodies.push(JSON.parse(init?.body ?? '{}'))
          return jsonFetch({ id: 's1' })
        }
        return streamingResponse([sse({ type: 'RUN_FINISHED' })])
      }),
    )
    const chat = useChatStore()

    await chat.send('查一下', { title: '追问 a.example. · upstream_timeout', traceId: 't1' })
    await chat.send('再查一次')

    // The label belongs to the session, so it is written once, at creation.
    expect(bodies).toEqual([
      { state: { title: '追问 a.example. · upstream_timeout', traceId: 't1' } },
    ])
  })

  it('titles a plain chat after the question that opened it', async () => {
    let created: unknown
    vi.stubGlobal(
      'fetch',
      vi.fn(async (path: string, init?: { body?: string }) => {
        if (path.endsWith('/sessions')) {
          created = JSON.parse(init?.body ?? '{}')
          return jsonFetch({ id: 's1' })
        }
        return streamingResponse([sse({ type: 'RUN_FINISHED' })])
      }),
    )
    const chat = useChatStore()

    const question = `这周的 ${'x'.repeat(60)} 成功率为什么掉了`
    await chat.send(question)

    expect(created).toEqual({ state: { title: question.slice(0, 48) } })
  })

  it('replays a stored session into ordered segments instead of flattened text', async () => {
    const events: SessionEvent[] = [
      { author: 'user', content: { role: 'user', parts: [{ text: '整体健康度如何' }] }, id: 'e0' },
      {
        author: 'doggy-dns-console',
        content: { role: 'model', parts: [{ thinking: '先查 KPI。' }] },
        id: 'e1',
      },
      {
        author: 'doggy-dns-console',
        content: {
          role: 'model',
          parts: [{ args: { windowMinutes: 60 }, id: 'tc_0', name: 'get_overview' }],
        },
        id: 'e2',
      },
      {
        author: 'doggy-dns-console',
        content: {
          role: 'model',
          parts: [
            { functionResponse: { name: 'get_overview', response: { queries: 128934 } }, id: 'tc_0' },
          ],
        },
        id: 'e3',
      },
      {
        author: 'doggy-dns-console',
        content: { role: 'model', parts: [{ text: '成功率 99.7%。' }] },
        id: 'e4',
      },
    ]
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        jsonFetch({
          id: 's1',
          appName: 'doggy-dns-console',
          userId: 'local',
          lastUpdateTime: 1,
          events,
          state: {},
        }),
      ),
    )
    const chat = useChatStore()
    await chat.load('s1')

    expect(chat.messages.map((message) => message.role)).toEqual(['user', 'assistant'])
    expect(kinds(chat.messages, 1)).toEqual(['thinking', 'tool', 'text'])
    const tool = chat.messages[1]?.segments[1]
    expect(tool?.kind === 'tool' && tool.args).toBe('{"windowMinutes":60}')
    expect(tool?.kind === 'tool' && JSON.parse(tool.result ?? '{}')).toMatchObject({ queries: 128934 })
  })
})
