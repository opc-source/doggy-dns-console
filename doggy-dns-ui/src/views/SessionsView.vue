<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { Plus } from '@lucide/vue'

import ErrorBanner from '@/components/ErrorBanner.vue'
import MessageBubble from '@/components/MessageBubble.vue'
import SessionList from '@/components/SessionList.vue'
import { apiGet } from '@/api/http'
import type { SessionSummary } from '@/api/types'
import { mapSessionEvents, useChatStore } from '@/stores/chat'

const appName = 'doggy-dns-console'
const userId = 'local'
const sessions = ref<SessionSummary[]>([])
const selected = ref<string | null>(null)
const detail = ref<SessionSummary | null>(null)
const error = ref<string | null>(null)
const chat = useChatStore()
const router = useRouter()

const messages = computed(() => (detail.value ? mapSessionEvents(detail.value.events) : []))

async function loadList() {
  try {
    sessions.value = await apiGet<SessionSummary[]>(
      `/api/apps/${appName}/users/${userId}/sessions`,
    )
    error.value = null
    if (!selected.value && sessions.value[0]) selected.value = sessions.value[0].id
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
})
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
      <SessionList :sessions="sessions" :selected="selected" @select="selected = $event" />
      <div class="panel flex min-h-[420px] flex-col overflow-hidden">
        <header class="border-b border-border px-4 py-3">
          <h2 class="text-sm font-medium">Transcript</h2>
        </header>
        <div class="min-h-0 flex-1 space-y-3 overflow-y-auto p-4">
          <div v-if="messages.length === 0" class="grid h-full place-items-center text-xs text-muted">
            选择左侧会话，或开始新对话。
          </div>
          <MessageBubble v-for="(message, index) in messages" :key="index" :message="message" />
        </div>
      </div>
    </section>
  </div>
</template>
