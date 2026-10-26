import { ref, watch } from 'vue'
import { defineStore } from 'pinia'

import { apiGet, apiPost } from '@/api/http'
import type { SessionEvent, SessionSummary } from '@/api/types'
import { useAgUiStream, type ToolCall } from '@/composables/useAgUiStream'

export interface ChatMessage {
  role: 'user' | 'assistant'
  content: string
  toolCalls?: ToolCall[]
}

export function mapSessionEvents(events: SessionEvent[]): ChatMessage[] {
  const messages: ChatMessage[] = []
  for (const event of events) {
    const text = (event.content?.parts ?? [])
      .map((part) => part.text)
      .filter((value): value is string => Boolean(value))
      .join('')
    if (!text) continue
    messages.push({
      role: event.author === 'user' ? 'user' : 'assistant',
      content: text,
    })
  }
  return messages
}

export const useChatStore = defineStore('chat', () => {
  const appName = 'doggy-dns-console'
  const userId = 'local'
  const sessionId = ref<string | null>(null)
  const messages = ref<ChatMessage[]>([])
  const error = ref<string | null>(null)
  const stream = useAgUiStream()

  async function ensureSession(): Promise<string> {
    if (sessionId.value) return sessionId.value
    const created = await apiPost<{ id: string }>(
      `/api/apps/${appName}/users/${userId}/sessions`,
      {},
    )
    sessionId.value = created.id
    return created.id
  }

  async function send(content: string) {
    error.value = null
    const id = await ensureSession()
    messages.value.push({ role: 'user', content })
    const assistant: ChatMessage = { role: 'assistant', content: '', toolCalls: [] }
    messages.value.push(assistant)

    const stopText = watch(stream.text, (value) => {
      assistant.content = value
    })
    const stopTools = watch(
      stream.toolCalls,
      (value) => {
        assistant.toolCalls = value.map((item) => ({ ...item }))
      },
      { deep: true },
    )
    try {
      await stream.send({ appName, userId, sessionId: id, message: content })
      assistant.content = stream.text.value
      if (stream.error.value) error.value = stream.error.value
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
    } finally {
      stopText()
      stopTools()
    }
  }

  function stop() {
    stream.stop()
  }

  async function load(id: string) {
    const session = await apiGet<SessionSummary>(
      `/api/apps/${appName}/users/${userId}/sessions/${id}`,
    )
    sessionId.value = session.id
    messages.value = mapSessionEvents(session.events)
  }

  function reset() {
    sessionId.value = null
    messages.value = []
    error.value = null
    stream.reset()
  }

  return { appName, userId, sessionId, messages, error, running: stream.running, send, stop, load, reset, ensureSession }
})
