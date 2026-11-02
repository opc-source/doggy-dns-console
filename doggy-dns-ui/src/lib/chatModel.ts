import type { SessionEvent, SessionEventPart } from '@/api/types'

export type ToolCallStatus = 'running' | 'done' | 'error'

export interface ThinkingSegment {
  kind: 'thinking'
  messageId: string
  text: string
}

export interface TextSegment {
  kind: 'text'
  messageId: string
  text: string
}

export interface ToolSegment {
  kind: 'tool'
  toolCallId: string
  name: string
  args: string
  result?: string
  status: ToolCallStatus
}

export type Segment = ThinkingSegment | TextSegment | ToolSegment

export interface ChatMessage {
  role: 'user' | 'assistant'
  segments: Segment[]
}

export interface AgUiEvent {
  type: string
  [key: string]: unknown
}

function deltaOf(event: AgUiEvent): string {
  return typeof event.delta === 'string' ? event.delta : ''
}

function findMessage(
  segments: Segment[],
  kind: 'text' | 'thinking',
  messageId: string,
): TextSegment | ThinkingSegment | undefined {
  return segments.find(
    (segment): segment is TextSegment | ThinkingSegment =>
      segment.kind === kind && segment.messageId === messageId,
  )
}

function findTool(segments: Segment[], toolCallId: string): ToolSegment | undefined {
  return segments.find(
    (segment): segment is ToolSegment => segment.kind === 'tool' && segment.toolCallId === toolCallId,
  )
}

/** A run that ends without answering an open call leaves the card pulsing forever. */
export function settleRunningTools(segments: Segment[], status: ToolCallStatus): void {
  for (const segment of segments) {
    if (segment.kind === 'tool' && segment.status === 'running') segment.status = status
  }
}

function appendMessage(segments: Segment[], event: AgUiEvent, kind: 'text' | 'thinking'): void {
  const messageId = String(event.messageId ?? '')
  const delta = deltaOf(event)
  const existing = findMessage(segments, kind, messageId)
  if (existing) {
    existing.text += delta
    return
  }
  // A `*_START` event carries no delta but fixes where the message goes.
  const created: Segment =
    kind === 'text'
      ? { kind: 'text', messageId, text: delta }
      : { kind: 'thinking', messageId, text: delta }
  segments.push(created)
}

function appendTool(segments: Segment[], event: AgUiEvent): ToolSegment {
  const toolCallId = String(event.toolCallId ?? '')
  const existing = findTool(segments, toolCallId)
  if (existing) return existing
  const created: ToolSegment = {
    kind: 'tool',
    toolCallId,
    name: String(event.toolCallName ?? 'tool'),
    args: '',
    status: 'running',
  }
  segments.push(created)
  return created
}

export function applyAgUiEvent(segments: Segment[], event: AgUiEvent): void {
  switch (event.type) {
    case 'TEXT_MESSAGE_START':
    case 'TEXT_MESSAGE_CONTENT':
    case 'TEXT_MESSAGE_CHUNK': {
      appendMessage(segments, event, 'text')
      break
    }
    // Only the message-level reasoning events name a segment: `REASONING_START` and
    // `REASONING_END` carry a separate `*-reasoning-phase-*` id.
    case 'REASONING_MESSAGE_START':
    case 'REASONING_MESSAGE_CONTENT':
    case 'REASONING_MESSAGE_CHUNK': {
      appendMessage(segments, event, 'thinking')
      break
    }
    case 'TOOL_CALL_START': {
      appendTool(segments, event)
      break
    }
    case 'TOOL_CALL_CHUNK': {
      appendTool(segments, event).args += deltaOf(event)
      break
    }
    case 'TOOL_CALL_ARGS': {
      // A producer may stream args without repeating the start; dropping the delta
      // would lose the call's input entirely.
      appendTool(segments, event).args += deltaOf(event)
      break
    }
    case 'TOOL_CALL_RESULT': {
      // A held call resumes in a later run, so a result can name a call this panel
      // never saw; rebuild it rather than dropping the payload.
      const call = appendTool(segments, event)
      call.result = String(event.content ?? '')
      call.status = 'done'
      break
    }
    case 'RUN_ERROR': {
      settleRunningTools(segments, 'error')
      break
    }
  }
}

function jsonOf(value: unknown): string {
  return typeof value === 'string' ? value : JSON.stringify(value ?? null)
}

function segmentFromPart(
  part: SessionEventPart,
  messageId: string,
): TextSegment | ThinkingSegment | ToolSegment | null {
  if (part.thinking) return { kind: 'thinking', messageId, text: part.thinking }
  if (part.text) return { kind: 'text', messageId, text: part.text }
  if (part.name && part.args !== undefined) {
    return {
      kind: 'tool',
      toolCallId: part.id ?? messageId,
      name: part.name,
      args: jsonOf(part.args),
      status: 'running',
    }
  }
  return null
}

function pushAssistant(messages: ChatMessage[]): ChatMessage {
  const created: ChatMessage = { role: 'assistant', segments: [] }
  messages.push(created)
  return created
}

export function messagesFromSessionEvents(events: SessionEvent[]): ChatMessage[] {
  const messages: ChatMessage[] = []
  let assistant: ChatMessage | null = null

  for (const [eventIndex, event] of events.entries()) {
    const eventId = event.id ?? String(eventIndex)
    const parts = event.content?.parts ?? []

    if (event.author === 'user') {
      const text = parts
        .map((part) => part.text)
        .filter((value): value is string => Boolean(value))
        .join('')
      if (text) {
        messages.push({ role: 'user', segments: [{ kind: 'text', messageId: `${eventId}-text`, text }] })
      }
      assistant = null
      continue
    }

    // The encoder sends every text (and every thinking) part of one event as a
    // single message at the first part's position, so replay merges them the same way.
    let openText: TextSegment | null = null
    let openThinking: ThinkingSegment | null = null
    parts.forEach((part, partIndex) => {
      const messageId = `${eventId}-p${partIndex}`
      if (part.functionResponse) {
        assistant ??= pushAssistant(messages)
        const response = part.functionResponse
        const call = findTool(assistant.segments, part.id ?? '')
        if (call) {
          call.result = jsonOf(response.response)
          call.status = 'done'
        } else {
          assistant.segments.push({
            kind: 'tool',
            toolCallId: part.id ?? messageId,
            name: response.name,
            args: '',
            result: jsonOf(response.response),
            status: 'done',
          })
        }
        return
      }
      const segment = segmentFromPart(part, messageId)
      if (!segment) return
      assistant ??= pushAssistant(messages)
      if (segment.kind === 'text') {
        if (openText) {
          openText.text += segment.text
          return
        }
        openText = segment
      } else if (segment.kind === 'thinking') {
        if (openThinking) {
          openThinking.text += segment.text
          return
        }
        openThinking = segment
      }
      assistant.segments.push(segment)
    })
  }

  return messages
}
