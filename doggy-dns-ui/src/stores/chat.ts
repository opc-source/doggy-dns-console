import { computed, reactive, ref } from 'vue'
import { defineStore } from 'pinia'

import { apiGet, apiPost } from '@/api/http'
import type { SessionSummary } from '@/api/types'
import { useAgUiStream } from '@/composables/useAgUiStream'
import {
  messagesFromSessionEvents,
  settleRunningTools,
  type ChatMessage,
  type Segment,
  type TextSegment,
} from '@/lib/chatModel'

export type { ChatMessage, Segment } from '@/lib/chatModel'

function userMessage(content: string): ChatMessage {
  const segment: TextSegment = { kind: 'text', messageId: '', text: content }
  return { role: 'user', segments: [segment] }
}

export const useChatStore = defineStore('chat', () => {
  const appName = 'doggy-dns-console'
  const userId = 'local'
  const sessionId = ref<string | null>(null)
  const messages = ref<ChatMessage[]>([])
  const error = ref<string | null>(null)
  const stream = useAgUiStream()
  const running = computed(() => stream.running.value)

  async function ensureSession(): Promise<string> {
    if (sessionId.value) return sessionId.value
    const created = await apiPost<{ id: string }>(
      `/api/apps/${appName}/users/${userId}/sessions`,
      {},
    )
    sessionId.value = created.id
    return created.id
  }

  async function send(content: string): Promise<boolean> {
    error.value = null
    let id: string
    try {
      id = await ensureSession()
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
      return false
    }
    messages.value.push(userMessage(content))
    const assistant = reactive<ChatMessage>({ role: 'assistant', segments: [] })
    messages.value.push(assistant)

    // The reducer mutates the array it is handed, so it has to receive the reactive
    // proxy — a plain `assistant.segments` reference would keep the UI still.
    const segments = assistant.segments as Segment[]
    try {
      await stream.send({ appName, userId, sessionId: id, message: content }, segments)
      if (stream.error.value) error.value = stream.error.value
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
    }
    // However this run ended — finished, failed, or superseded by the next
    // question — a call it never answered will not be answered in this message.
    settleRunningTools(segments, 'error')
    return true
  }

  function stop() {
    const last = messages.value[messages.value.length - 1]
    stream.stop()
    // A stopped run never answers its open calls; leaving them pulsing would
    // claim work that has already gone away.
    if (last?.role === 'assistant') settleRunningTools(last.segments, 'error')
  }

  async function load(id: string) {
    stream.stop()
    const session = await apiGet<SessionSummary>(
      `/api/apps/${appName}/users/${userId}/sessions/${id}`,
    )
    sessionId.value = session.id
    messages.value = messagesFromSessionEvents(session.events)
  }

  function reset() {
    // The transcript is going away, so the run writing into it has to go with it.
    stream.stop()
    sessionId.value = null
    messages.value = []
    error.value = null
  }

  return {
    appName,
    userId,
    sessionId,
    messages,
    error,
    running,
    send,
    stop,
    load,
    reset,
    ensureSession,
  }
})
