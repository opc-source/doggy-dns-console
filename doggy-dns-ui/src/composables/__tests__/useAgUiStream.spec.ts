import { describe, expect, it, vi } from 'vitest'

import { useAgUiStream } from '@/composables/useAgUiStream'

function encoder() {
  return new TextEncoder()
}

function sse(value: unknown) {
  return `data: ${JSON.stringify(value)}\n\n`
}

function streamingFetch(chunks: string[]) {
  return vi.fn(async () => {
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        for (const chunk of chunks) controller.enqueue(encoder().encode(chunk))
        controller.close()
      },
    })
    return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } })
  })
}

const request = { appName: 'a', userId: 'u', sessionId: 's', message: 'hi' }

describe('useAgUiStream', () => {
  it('parses events split across chunks', async () => {
    vi.stubGlobal(
      'fetch',
      streamingFetch([
        'data: {"type":"TEXT_MESSAGE_CONTENT","delta":"你',
        '好"}\n\n',
        'data: {"type":"TOOL_CALL_START","toolCallId":"c1","toolCallName":"get_overview"}\n\n',
        'data: {"type":"TOOL_CALL_ARGS","toolCallId":"c1","delta":"{\\"windowMinutes\\":60}"}\n\n',
        'data: {"type":"TOOL_CALL_RESULT","toolCallId":"c1","content":"{\\"ok\\":true}"}\n\n',
        'data: {"type":"RUN_FINISHED"}\n\n',
      ]),
    )
    const stream = useAgUiStream()
    await stream.send(request)
    expect(stream.text.value).toBe('你好')
    expect(stream.toolCalls.value[0]?.name).toBe('get_overview')
    expect(stream.toolCalls.value[0]?.status).toBe('done')
    expect(stream.running.value).toBe(false)
  })

  it('parses streaming chunk events from a real model', async () => {
    vi.stubGlobal(
      'fetch',
      streamingFetch([
        sse({ type: 'TEXT_MESSAGE_CHUNK', delta: '你' }),
        sse({ type: 'TEXT_MESSAGE_CHUNK', delta: '好' }),
        sse({
          type: 'TOOL_CALL_CHUNK',
          toolCallId: 'c2',
          toolCallName: 'openobserve_search_sql',
          delta: '{"sql":',
        }),
        sse({ type: 'TOOL_CALL_CHUNK', toolCallId: 'c2', delta: '"SELECT 1"}' }),
        sse({ type: 'RUN_FINISHED' }),
      ]),
    )
    const stream = useAgUiStream()
    await stream.send(request)
    expect(stream.text.value).toBe('你好')
    expect(stream.toolCalls.value[0]?.name).toBe('openobserve_search_sql')
    expect(stream.toolCalls.value[0]?.args).toBe('{"sql":"SELECT 1"}')
  })

  it('abort keeps partial text', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => {
        const body = new ReadableStream<Uint8Array>({
          start(controller) {
            controller.enqueue(
              encoder().encode('data: {"type":"TEXT_MESSAGE_CONTENT","delta":"部分"}\n\n'),
            )
          },
        })
        return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } })
      }),
    )
    const stream = useAgUiStream()
    const pending = stream.send(request)
    await vi.waitFor(() => expect(stream.text.value).toBe('部分'))
    stream.stop()
    await pending
    expect(stream.text.value).toBe('部分')
  })
})
