<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import FailureDrawer from '@/components/FailureDrawer.vue'

const route = useRoute()
const router = useRouter()

const DEFAULT_WINDOW_MINUTES = 60

function firstQueryValue(value: unknown): string | null {
  if (Array.isArray(value)) {
    const first = value[0]
    return typeof first === 'string' ? first : null
  }
  return typeof value === 'string' ? value : null
}

const traceId = computed(() => String(route.params.traceId ?? ''))

const windowMinutes = computed(() => {
  const raw = firstQueryValue(route.query.windowMinutes)
  if (raw === null) return DEFAULT_WINDOW_MINUTES
  const parsed = Number(raw)
  return Number.isInteger(parsed) && parsed > 0 ? parsed : DEFAULT_WINDOW_MINUTES
})

const analysisId = computed(() => {
  const raw = firstQueryValue(route.query.analysis)
  return raw ? raw : undefined
})

function close() {
  void router.push({ name: 'overview' })
}
</script>

<template>
  <main class="mx-auto max-w-[1440px] p-5 lg:p-7" aria-label="Failure investigation">
    <FailureDrawer
      :trace-id="traceId"
      :window-minutes="windowMinutes"
      :analysis-id="analysisId"
      @close="close"
    />
  </main>
</template>
