<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import type { InvestigationFilter } from '@/api/types'
import FailureDrawer from '@/components/FailureDrawer.vue'

const route = useRoute()
const router = useRouter()

const DEFAULT_WINDOW_MINUTES = 60
// The backend clamps the evidence window to this range; parsing here keeps the
// drawer's request (and the restore link) inside it instead of earning a 400.
const MIN_WINDOW_MINUTES = 5
const MAX_WINDOW_MINUTES = 1440
const POSITIVE_INTEGER = /^\d+$/

function firstQueryValue(value: unknown): string | null {
  if (Array.isArray(value)) {
    const first = value[0]
    return typeof first === 'string' ? first : null
  }
  return typeof value === 'string' ? value : null
}

const traceId = computed(() => String(route.params.traceId ?? ''))

const windowMinutes = computed(() => {
  const raw = firstQueryValue(route.query.windowMinutes)?.trim()
  if (!raw || !POSITIVE_INTEGER.test(raw)) return DEFAULT_WINDOW_MINUTES
  const parsed = Number(raw)
  if (Number.isNaN(parsed) || parsed <= 0) return DEFAULT_WINDOW_MINUTES
  // Clamp before the infinity check: a value too large for a finite float is
  // still "too large", not invalid.
  return Math.min(Math.max(parsed, MIN_WINDOW_MINUTES), MAX_WINDOW_MINUTES)
})

// Task 11 replaces the windowMinutes clamp block above wholesale; until then the
// deep link still carries a relative window, so bridge it to an absolute bracket
// for the drawer. `now` is captured once so the span is exactly the window.
const filter = computed<InvestigationFilter>(() => {
  const now = Date.now()
  return {
    from: now - windowMinutes.value * 60_000,
    to: now,
    domain: null,
    responseCode: null,
    instance: null,
  }
})

const analysisId = computed(() => {
  const raw = firstQueryValue(route.query.analysis)?.trim()
  return raw ? raw : undefined
})

function close() {
  void router.push({ name: 'overview' })
}
</script>

<template>
  <section class="mx-auto max-w-[1440px] p-5 lg:p-7" aria-label="Failure investigation">
    <FailureDrawer
      :trace-id="traceId"
      :filter="filter"
      :analysis-id="analysisId"
      @close="close"
    />
  </section>
</template>
