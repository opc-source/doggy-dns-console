<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'
import { Plus } from '@lucide/vue'

import ErrorBanner from '@/components/ErrorBanner.vue'
import MessageBubble from '@/components/MessageBubble.vue'
import SessionList from '@/components/SessionList.vue'
import { apiGet } from '@/api/http'
import type { SessionSummary } from '@/api/types'
import { messagesFromSessionEvents } from '@/lib/chatModel'
import { sessionFailureTarget, sessionTitle } from '@/lib/sessionState'
import { useChatStore } from '@/stores/chat'

const appName = 'doggy-dns-console'
const userId = 'local'
const sessions = ref<SessionSummary[]>([])
const detail = ref<SessionSummary | null>(null)
const error = ref<string | null>(null)
const chat = useChatStore()
const route = useRoute()
const router = useRouter()

// The address owns the selection: a transcript is worth refreshing, sharing and
// going Back to, and a handoff only reaches the failure it came from by id.
const selected = computed(() => {
  const value = route.params.sessionId
  return typeof value === 'string' && value !== '' ? value : null
})

// The list entry knows the label the session was created with; the detail
// response knows it too, and covers a session deep-linked from elsewhere.
const labelled = computed(
  () => sessions.value.find((session) => session.id === selected.value) ?? detail.value,
)
const title = computed(() => (labelled.value ? sessionTitle(labelled.value) : ''))
const failureTarget = computed(() =>
  labelled.value ? sessionFailureTarget(labelled.value) : null,
)

const messages = computed(() => (detail.value ? messagesFromSessionEvents(detail.value.events) : []))

function selectSession(id: string) {
  void router.push({ name: 'sessions', params: { sessionId: id } })
}

async function loadList() {
  try {
    sessions.value = await apiGet<SessionSummary[]>(
      `/api/apps/${appName}/users/${userId}/sessions`,
    )
    error.value = null
    const first = sessions.value[0]
    // Only a sessions address that names nobody may be filled in. While a
    // navigation is still in flight the route is not `sessions` at all, and
    // replacing then would overwrite the session the operator asked for.
    if (!selected.value && route.name === 'sessions' && first) {
      // A replace, not a push: the bare list is not a page to come Back to, and
      // the address should not keep pointing past the transcript on screen.
      void router.replace({ name: 'sessions', params: { sessionId: first.id } })
    }
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  }
}

async function loadDetail(id: string) {
  try {
    detail.value = await apiGet<SessionSummary>(
      `/api/apps/${appName}/users/${userId}/sessions/${id}`,
    )
    error.value = null
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  }
}

function newChat() {
  chat.reset()
  void router.push('/')
}

onMounted(loadList)
watch(selected, (id) => {
  if (id) void loadDetail(id)
}, { immediate: true })
</script>

<template>
  <div class="mx-auto flex h-full max-w-[1440px] flex-col gap-4 p-5 lg:p-7">
    <header class="flex flex-wrap items-end justify-between gap-4">
      <div>
        <p class="mono text-[11px] uppercase tracking-[0.18em] text-accent">history</p>
        <h1 class="mt-1 text-2xl font-semibold tracking-tight lg:text-3xl">Sessions</h1>
        <p class="mt-1 text-sm text-muted">Replay previous analyses. Stored in local SQLite.</p>
      </div>
      <button class="focus-ring flex items-center gap-2 rounded-xl bg-accent px-3 py-2 text-xs font-medium text-bg hover:opacity-90" @click="newChat">
        <Plus :size="13" /> New chat
      </button>
    </header>

    <ErrorBanner v-if="error" :message="error" @retry="loadList" />

    <section class="grid min-h-0 flex-1 grid-cols-1 gap-3 lg:grid-cols-[300px_1fr]">
      <SessionList :sessions="sessions" :selected="selected" @select="selectSession" />
      <div class="panel flex min-h-[420px] flex-col overflow-hidden">
        <header class="flex items-center gap-3 border-b border-border px-4 py-3">
          <h2 class="shrink-0 text-sm font-medium">Transcript</h2>
          <p data-test="selected-session" class="min-w-0 flex-1 truncate text-xs text-muted">{{ title }}</p>
          <RouterLink
            v-if="failureTarget"
            data-test="failure-jump"
            :to="failureTarget"
            class="focus-ring shrink-0 rounded-lg border border-border px-2 py-1 text-[11px] text-muted hover:bg-surface-2 hover:text-text"
          >
            回到失败详情
          </RouterLink>
        </header>
        <div class="min-h-0 flex-1 space-y-3 overflow-y-auto p-4">
          <div v-if="messages.length === 0" class="grid h-full place-items-center text-xs text-muted">
            选择左侧会话，或开始新对话。
          </div>
          <!-- Keyed to the session so a switch never inherits the previous transcript's state. -->
          <MessageBubble
            v-for="(message, index) in messages"
            :key="`${selected}:${index}`"
            :message="message"
          />
        </div>
      </div>
    </section>
  </div>
</template>
