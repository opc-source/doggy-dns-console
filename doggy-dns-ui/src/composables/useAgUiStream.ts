import { ref } from 'vue'

export interface ToolCall {
  id: string
  name: string
  args: string
  result?: string
  status: 'running' | 'done' | 'error'
}

export interface AgUiEvent {
  type: string
  [key: string]: unknown
}

export interface SendRequest {
  appName: string
  userId: string
  sessionId: string
  message: string
}

export function useAgUiStream() {
  const text = ref('')
  const toolCalls = ref<ToolCall[]>([])
  const events = ref<AgUiEvent[]>([])
  const running = ref(false)
  const error = ref<string | null>(null)
  let controller: AbortController | null = null
  let activeReader: ReadableStreamDefaultReader<Uint8Array> | null = null
  let aborted = false

  function reset() {
    text.value = ''
    toolCalls.value = []
    events.value = []
    error.value = null
  }

  function stop() {
    aborted = true
    controller?.abort()
    void activeReader?.cancel().catch(() => undefined)
    running.value = false
  }

  async function send(request: SendRequest) {
    reset()
    running.value = true
    aborted = false
    controller = new AbortController()
    try {
      const response = await fetch('/api/run_sse', {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          accept: 'text/event-stream',
          'x-adk-ui-protocol': 'ag_ui',
          'x-adk-ui-transport': 'protocol_native',
        },
        body: JSON.stringify({
          appName: request.appName,
          userId: request.userId,
          sessionId: request.sessionId,
          newMessage: { role: 'user', parts: [{ text: request.message }] },
          uiProtocol: 'ag_ui',
          uiTransport: 'protocol_native',
        }),
        signal: controller.signal,
      })
      if (!response.ok) {
        const body = await response.text()
        throw new Error(body || `agent returned ${response.status}`)
      }
      if (!response.body) throw new Error('agent returned no stream')

      const reader = response.body.getReader()
      activeReader = reader
      const decoder = new TextDecoder()
      let buffer = ''
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true })
        buffer = buffer.replace(/\r\n/g, '\n')
        let boundary = buffer.indexOf('\n\n')
        while (boundary >= 0) {
          const block = buffer.slice(0, boundary)
          buffer = buffer.slice(boundary + 2)
          handleBlock(block)
          boundary = buffer.indexOf('\n\n')
        }
        if (aborted) {
          await reader.cancel().catch(() => undefined)
          break
        }
      }
    } catch (cause) {
      if (!aborted) {
        error.value = cause instanceof Error ? cause.message : String(cause)
      }
    } finally {
      running.value = false
      controller = null
      activeReader = null
    }
  }

  function handleBlock(block: string) {
    const data = block
      .split('\n')
      .filter((line) => line.startsWith('data:'))
      .map((line) => line.slice(5).trimStart())
      .join('\n')
    if (!data || data === '[DONE]') return
    let event: AgUiEvent
    try {
      event = JSON.parse(data) as AgUiEvent
    } catch {
      return
    }
    events.value.push(event)
    apply(event)
  }

  function apply(event: AgUiEvent) {
    switch (event.type) {
      case 'TEXT_MESSAGE_CONTENT': {
        const delta = typeof event.delta === 'string' ? event.delta : ''
        text.value += delta
        break
      }
      case 'TOOL_CALL_START': {
        const id = String(event.toolCallId ?? event.tool_call_id ?? '')
        const name = String(event.toolCallName ?? event.tool_call_name ?? 'tool')
        toolCalls.value.push({ id, name, args: '', status: 'running' })
        break
      }
      case 'TOOL_CALL_ARGS': {
        const id = String(event.toolCallId ?? event.tool_call_id ?? '')
        const delta = String(event.delta ?? '')
        const call = toolCalls.value.find((item) => item.id === id)
        if (call) call.args += delta
        break
      }
      case 'TOOL_CALL_RESULT': {
        const id = String(event.toolCallId ?? event.tool_call_id ?? '')
        const call = toolCalls.value.find((item) => item.id === id)
        if (call) {
          call.result = String(event.content ?? event.result ?? '')
          call.status = 'done'
        }
        break
      }
      case 'RUN_ERROR': {
        error.value = String(event.message ?? event.error ?? 'agent run failed')
        running.value = false
        break
      }
      case 'RUN_FINISHED': {
        running.value = false
        break
      }
    }
  }

  return { text, toolCalls, events, running, error, send, stop, reset }
}
