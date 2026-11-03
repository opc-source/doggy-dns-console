import { onMounted, onUnmounted, ref, watch, type Ref } from 'vue'

import { apiGet } from '@/api/http'
import type { Instance, InstanceDetail } from '@/api/types'
import { type FilterDraft, toRequestQuery } from '@/lib/failureFilter'

export function useInstances() {
  const instances = ref<Instance[]>([])
  const error = ref<string | null>(null)
  let timer: ReturnType<typeof setInterval> | null = null

  async function refresh() {
    try {
      instances.value = await apiGet<Instance[]>('/api/instances')
      error.value = null
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
    }
  }

  onMounted(() => {
    void refresh()
    timer = setInterval(refresh, 15_000)
  })
  onUnmounted(() => {
    if (timer) clearInterval(timer)
  })
  return { instances, error, refresh }
}

export function useInstanceDetail(id: Ref<string | null>, draft: Ref<FilterDraft>) {
  const detail = ref<InstanceDetail | null>(null)
  const error = ref<string | null>(null)
  const loading = ref(false)

  async function refresh() {
    if (!id.value) {
      detail.value = null
      return
    }
    loading.value = true
    try {
      // The bracket travels with the request: the failure number is a range
      // answer, everything else in the detail is live MCP data.
      const query = new URLSearchParams(toRequestQuery(draft.value)).toString()
      detail.value = await apiGet<InstanceDetail>(
        `/api/instances/${encodeURIComponent(id.value)}?${query}`,
      )
      error.value = null
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
    } finally {
      loading.value = false
    }
  }

  // `immediate` rather than `onMounted`: this composable owns no timer, so it
  // needs no lifecycle hook — and that is also why its spec can call it without
  // a host component, unlike `useOverview`.
  watch([id, draft], () => void refresh(), { immediate: true })
  return { detail, error, loading, refresh }
}
