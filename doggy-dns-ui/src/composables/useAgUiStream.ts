import { ref } from 'vue'

import { applyAgUiEvent, type AgUiEvent, type Segment } from '@/lib/chatModel'

export interface SendRequest {
  appName: string
  userId: string
  sessionId: string
  message: string
}

export function useAgUiStream() {
  const running = ref(false)
  const error = ref<string | null>(null)
  // Only the newest run owns `running`, the abort handle and `stop()`. Without a
  // generation, a retired run's cleanup would strip them from the live one.
  let generation = 0
  let active: {
    controller: AbortController
    reader?: ReadableStreamDefaultReader<Uint8Array>
  } | null = null

  function retireActive() {
    const retiring = active
    active = null
    retiring?.controller.abort()
    void retiring?.reader?.cancel().catch(() => undefined)
  }

  function stop() {
    generation += 1
    retireActive()
    running.value = false
  }

  async function send(request: SendRequest, segments: Segment[]) {
    retireActive()
    const mine = ++generation
    const controller = new AbortController()
    active = { controller }
    error.value = null
    running.value = true
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
      if (mine === generation) active = { controller, reader }
      const decoder = new TextDecoder()
      let buffer = ''
      while (true) {
        const { done, value } = await reader.read()
        if (mine !== generation) {
          void reader.cancel().catch(() => undefined)
          break
        }
        if (done) break
        buffer += decoder.decode(value, { stream: true })
        buffer = buffer.replace(/\r\n/g, '\n')
        let boundary = buffer.indexOf('\n\n')
        while (boundary >= 0) {
          const block = buffer.slice(0, boundary)
          buffer = buffer.slice(boundary + 2)
          handleBlock(block, segments)
          boundary = buffer.indexOf('\n\n')
        }
      }
    } catch (cause) {
      if (mine === generation) {
        error.value = cause instanceof Error ? cause.message : String(cause)
      }
    } finally {
      if (mine === generation) {
        running.value = false
        active = null
      }
    }
  }

  function handleBlock(block: string, segments: Segment[]) {
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
    applyAgUiEvent(segments, event)
    if (event.type === 'RUN_ERROR') {
      error.value = String(event.message ?? 'agent run failed')
      running.value = false
    } else if (event.type === 'RUN_FINISHED') {
      running.value = false
    }
  }

  return { running, error, send, stop }
}
