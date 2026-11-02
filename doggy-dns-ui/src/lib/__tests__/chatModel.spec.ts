import { describe, expect, it } from 'vitest'

import { applyAgUiEvent, messagesFromSessionEvents, type Segment } from '@/lib/chatModel'
import type { SessionEvent, SessionEventPart } from '@/api/types'

function feed(events: Array<Record<string, unknown>>): Segment[] {
  const segments: Segment[] = []
  for (const item of events) applyAgUiEvent(segments, item as never)
  return segments
}

function statusAt(segments: Segment[], index: number) {
  const segment = segments[index]
  return segment?.kind === 'tool' ? segment.status : undefined
}

describe('applyAgUiEvent', () => {
  it('keeps a text segment and a later tool segment in arrival order', () => {
    const segments = feed([
      { type: 'TEXT_MESSAGE_START', messageId: 'm1' },
      { type: 'TEXT_MESSAGE_CONTENT', messageId: 'm1', delta: '先看整体。' },
      { type: 'TEXT_MESSAGE_END', messageId: 'm1' },
      { type: 'TOOL_CALL_START', toolCallId: 'c1', toolCallName: 'get_overview' },
      { type: 'TOOL_CALL_ARGS', toolCallId: 'c1', delta: '{"windowMinutes":60}' },
      { type: 'TOOL_CALL_RESULT', toolCallId: 'c1', content: '{"ok":true}' },
      { type: 'TEXT_MESSAGE_START', messageId: 'm2' },
      { type: 'TEXT_MESSAGE_CONTENT', messageId: 'm2', delta: '成功率 99.7%。' },
    ])

    expect(segments.map((segment) => segment.kind)).toEqual(['text', 'tool', 'text'])
    expect(segments[0]).toMatchObject({ messageId: 'm1', text: '先看整体。' })
    expect(segments[1]).toMatchObject({
      toolCallId: 'c1',
      name: 'get_overview',
      args: '{"windowMinutes":60}',
      result: '{"ok":true}',
      status: 'done',
    })
    expect(segments[2]).toMatchObject({ messageId: 'm2', text: '成功率 99.7%。' })
  })

  it('interleaves reasoning between text and tools without a stray phase segment', () => {
    // The server names REASONING_START/END with a separate `*-reasoning-phase-*`
    // id, so only the message-level events may open a segment.
    const segments = feed([
      { type: 'REASONING_START', messageId: 'ev-1-reasoning-phase-0' },
      { type: 'REASONING_MESSAGE_START', messageId: 'ev-1-reasoning-0', role: 'assistant' },
      { type: 'REASONING_MESSAGE_CONTENT', messageId: 'ev-1-reasoning-0', delta: '先查 KPI。' },
      { type: 'REASONING_MESSAGE_END', messageId: 'ev-1-reasoning-0' },
      { type: 'REASONING_END', messageId: 'ev-1-reasoning-phase-0' },
      { type: 'TOOL_CALL_START', toolCallId: 'c1', toolCallName: 'get_overview' },
      { type: 'REASONING_MESSAGE_CHUNK', messageId: 'ev-2-reasoning-0', delta: '再看分组。' },
      { type: 'TEXT_MESSAGE_CHUNK', messageId: 'ev-2-text-0', delta: '结论。' },
    ])

    expect(segments.map((segment) => segment.kind)).toEqual(['thinking', 'tool', 'thinking', 'text'])
    expect(segments[0]).toMatchObject({ messageId: 'ev-1-reasoning-0', text: '先查 KPI。' })
    expect(segments[2]).toMatchObject({ messageId: 'ev-2-reasoning-0', text: '再看分组。' })
    expect(segments[3]).toMatchObject({ messageId: 'ev-2-text-0', text: '结论。' })
  })

  it('creates a tool segment from a chunk that carries no start', () => {
    const segments = feed([
      { type: 'TOOL_CALL_CHUNK', toolCallId: 'c2', toolCallName: 'openobserve_search_sql', delta: '{"sql":' },
      { type: 'TOOL_CALL_CHUNK', toolCallId: 'c2', delta: '"SELECT 1"}' },
    ])

    expect(segments).toHaveLength(1)
    expect(segments[0]).toMatchObject({
      kind: 'tool',
      name: 'openobserve_search_sql',
      args: '{"sql":"SELECT 1"}',
      status: 'running',
    })
  })

  it('keeps args that arrive for a call with no start event', () => {
    const segments = feed([
      { type: 'TOOL_CALL_ARGS', toolCallId: 'c9', delta: '{"windowMinutes":' },
      { type: 'TOOL_CALL_ARGS', toolCallId: 'c9', delta: '60}' },
    ])

    expect(segments).toHaveLength(1)
    expect(segments[0]).toMatchObject({ kind: 'tool', toolCallId: 'c9', args: '{"windowMinutes":60}' })
  })

  it('keeps a result that arrives for a call from a previous run', () => {
    const segments = feed([
      { type: 'TOOL_CALL_RESULT', toolCallId: 'held-1', toolCallName: 'get_config', content: '{"pid":1}' },
    ])

    expect(segments).toHaveLength(1)
    expect(segments[0]).toMatchObject({ kind: 'tool', name: 'get_config', result: '{"pid":1}', status: 'done' })
  })

  it('marks unfinished tool calls as errored when the run fails', () => {
    const segments = feed([
      { type: 'TOOL_CALL_START', toolCallId: 'c1', toolCallName: 'get_overview' },
      { type: 'TOOL_CALL_RESULT', toolCallId: 'c2', toolCallName: 'get_status', content: '{}' },
      { type: 'RUN_ERROR', message: 'model unavailable' },
    ])

    expect(statusAt(segments, 0)).toBe('error')
    expect(statusAt(segments, 1)).toBe('done')
  })
})

// The event shapes below are copied from a live `/sessions/{id}` response of the
// demo console: parts are untagged, so a call is `{args,id,name}` and a result is
// `{functionResponse,id}`.
function modelEvent(id: string, parts: SessionEventPart[]): SessionEvent {
  return { author: 'doggy-dns-console', content: { role: 'model', parts }, id }
}

describe('messagesFromSessionEvents', () => {
  it('collapses one run into an assistant message ordered call, result, answer', () => {
    const messages = messagesFromSessionEvents([
      { author: 'user', content: { role: 'user', parts: [{ text: '整体健康度如何' }] }, id: 'e0' },
      modelEvent('e1', [{ args: { windowMinutes: 60 }, id: 'tc_0', name: 'get_overview' }]),
      modelEvent('e2', [{ functionResponse: { name: 'get_overview', response: { queries: 128934 } }, id: 'tc_0' }]),
      modelEvent('e3', [{ text: '成功率 99.7%。' }]),
    ])

    expect(messages.map((message) => message.role)).toEqual(['user', 'assistant'])
    expect(messages[0]?.segments[0]).toMatchObject({ kind: 'text', text: '整体健康度如何' })
    const segments = messages[1]?.segments ?? []
    expect(segments.map((segment) => segment.kind)).toEqual(['tool', 'text'])
    expect(segments[0]).toMatchObject({
      toolCallId: 'tc_0',
      name: 'get_overview',
      args: '{"windowMinutes":60}',
      status: 'done',
    })
    const call = segments[0]
    if (call?.kind !== 'tool') throw new Error('expected a tool segment')
    expect(JSON.parse(call.result ?? '{}')).toMatchObject({ queries: 128934 })
  })

  it('keeps thinking ahead of the answer it produced', () => {
    const messages = messagesFromSessionEvents([
      modelEvent('e1', [{ thinking: '先查 KPI。' }, { text: '成功率 99.7%。' }]),
    ])

    const segments = messages[0]?.segments ?? []
    expect(segments.map((segment) => segment.kind)).toEqual(['thinking', 'text'])
  })

  it('joins an event\'s parts into one message the way the live stream does', () => {
    // The encoder sends every text part of one event as a single message, so a
    // replay must not split them into separate bubbles.
    const messages = messagesFromSessionEvents([
      modelEvent('e1', [
        { text: '成功率 99.7%。' },
        { thinking: '先看分组。' },
        { text: '建议检查上游转发。' },
      ]),
    ])

    const segments = messages[0]?.segments ?? []
    expect(segments.map((segment) => segment.kind)).toEqual(['text', 'thinking'])
    expect(segments[0]).toMatchObject({ text: '成功率 99.7%。建议检查上游转发。' })
    expect(segments[1]).toMatchObject({ text: '先看分组。' })
  })

  it('drops events that carry nothing renderable', () => {
    expect(messagesFromSessionEvents([modelEvent('e1', [{}])])).toEqual([])
  })
})
