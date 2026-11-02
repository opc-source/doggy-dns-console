import { describe, expect, it, vi } from 'vitest'

import { useAgUiStream } from '@/composables/useAgUiStream'
import type { Segment, ToolSegment } from '@/lib/chatModel'

function encoder() {
  return new TextEncoder()
}

function sse(value: unknown) {
  return `data: ${JSON.stringify(value)}\n\n`
}

function streamingFetch(chunks: string[]) {
  return vi.fn<() => Promise<Response>>(async () => {
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        for (const chunk of chunks) controller.enqueue(encoder().encode(chunk))
        controller.close()
      },
    })
    return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } })
  })
}

function textAt(segments: Segment[], index: number): string | null {
  const segment = segments[index]
  return segment?.kind === 'text' ? segment.text : null
}

function toolAt(segments: Segment[], index: number): ToolSegment | null {
  const segment = segments[index]
  return segment?.kind === 'tool' ? segment : null
}

const request = { appName: 'a', userId: 'u', sessionId: 's', message: 'hi' }

/** A stream the test feeds by hand, so two runs can genuinely overlap. */
function openStream() {
  let push!: (chunk: string) => void
  let close!: () => void
  let cancelled = false
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      // A retired run cancels its stream, so the test's later feeds are no-ops.
      push = (chunk) => {
        try {
          controller.enqueue(encoder().encode(chunk))
        } catch {
          // The run was retired mid-stream; the feed simply never arrives.
        }
      }
      close = () => {
        try {
          controller.close()
        } catch {
          // Already cancelled by the run being retired.
        }
      }
    },
    cancel() {
      cancelled = true
    },
  })
  return {
    body,
    push,
    close,
    wasCancelled: () => cancelled,
  }
}

function fetchInOrder(bodies: ReadableStream<Uint8Array>[]) {
  return vi.fn<() => Promise<Response>>(async () => {
    const body = bodies.shift()
    if (!body) throw new Error('no stream left for this run')
    return new Response(body, {
      status: 200,
      headers: { 'content-type': 'text/event-stream' },
    })
  })
}

describe('useAgUiStream', () => {
  it('parses events split across chunks into ordered segments', async () => {
    vi.stubGlobal(
      'fetch',
      streamingFetch([
        'data: {"type":"TEXT_MESSAGE_CONTENT","messageId":"m1","delta":"你',
        '好"}\n\n',
        'data: {"type":"TOOL_CALL_START","toolCallId":"c1","toolCallName":"get_overview"}\n\n',
        'data: {"type":"TOOL_CALL_ARGS","toolCallId":"c1","delta":"{\\"windowMinutes\\":60}"}\n\n',
        'data: {"type":"TOOL_CALL_RESULT","toolCallId":"c1","content":"{\\"ok\\":true}"}\n\n',
        'data: {"type":"RUN_FINISHED"}\n\n',
      ]),
    )
    const segments: Segment[] = []
    const stream = useAgUiStream()
    await stream.send(request, segments)

    expect(segments.map((segment) => segment.kind)).toEqual(['text', 'tool'])
    expect(textAt(segments, 0)).toBe('你好')
    expect(toolAt(segments, 1)?.name).toBe('get_overview')
    expect(toolAt(segments, 1)?.status).toBe('done')
    expect(stream.running.value).toBe(false)
  })

  it('parses streaming chunk events from a real model', async () => {
    vi.stubGlobal(
      'fetch',
      streamingFetch([
        sse({ type: 'TEXT_MESSAGE_CHUNK', messageId: 'm1', delta: '你' }),
        sse({ type: 'TEXT_MESSAGE_CHUNK', messageId: 'm1', delta: '好' }),
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
    const segments: Segment[] = []
    const stream = useAgUiStream()
    await stream.send(request, segments)

    expect(textAt(segments, 0)).toBe('你好')
    expect(toolAt(segments, 1)?.name).toBe('openobserve_search_sql')
    expect(toolAt(segments, 1)?.args).toBe('{"sql":"SELECT 1"}')
  })

  it('keeps the text streamed before a stop', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => {
        const body = new ReadableStream<Uint8Array>({
          start(controller) {
            controller.enqueue(
              encoder().encode(
                'data: {"type":"TEXT_MESSAGE_CONTENT","messageId":"m1","delta":"部分"}\n\n',
              ),
            )
          },
        })
        return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } })
      }),
    )
    const segments: Segment[] = []
    const stream = useAgUiStream()
    const pending = stream.send(request, segments)
    await vi.waitFor(() => expect(textAt(segments, 0)).toBe('部分'))
    stream.stop()
    await pending
    expect(textAt(segments, 0)).toBe('部分')
  })

  it('leaves the run state with the newest question when an older run lands', async () => {
    const older = openStream()
    const newer = openStream()
    vi.stubGlobal('fetch', fetchInOrder([older.body, newer.body]))
    const first: Segment[] = []
    const second: Segment[] = []
    const stream = useAgUiStream()

    const runA = stream.send(request, first)
    await vi.waitFor(() => expect(stream.running.value).toBe(true))
    // A 继续追问 handoff can start while the previous answer is still streaming.
    const runB = stream.send(request, second)
    older.close()
    await runA

    // The retired run must not switch off the spinner the live one still owns.
    expect(stream.running.value).toBe(true)

    newer.push(sse({ type: 'TEXT_MESSAGE_CONTENT', messageId: 'm2', delta: '新答案' }))
    newer.push(sse({ type: 'RUN_FINISHED' }))
    newer.close()
    await runB

    expect(stream.running.value).toBe(false)
    expect(textAt(second, 0)).toBe('新答案')
  })

  it('cancels the run that is actually streaming when an older one already finished', async () => {
    const older = openStream()
    const newer = openStream()
    vi.stubGlobal('fetch', fetchInOrder([older.body, newer.body]))
    const stream = useAgUiStream()

    const runA = stream.send(request, [])
    await vi.waitFor(() => expect(stream.running.value).toBe(true))
    const runB = stream.send(request, [])
    older.close()
    await runA

    stream.stop()
    await vi.waitFor(() => expect(newer.wasCancelled()).toBe(true))
    newer.close()
    await runB

    expect(stream.running.value).toBe(false)
    expect(stream.error.value).toBeNull()
  })
})
