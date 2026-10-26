import { onMounted, onUnmounted, ref, watch } from 'vue'

import { ApiError, apiGet } from '@/api/http'
import type { Overview } from '@/api/types'

export function useOverview(initialWindow = 60) {
  const windowMinutes = ref(initialWindow)
  const data = ref<Overview | null>(null)
  const error = ref<ApiError | null>(null)
  const loading = ref(false)
  const lastUpdated = ref<number | null>(null)
  let timer: ReturnType<typeof setInterval> | null = null

  async function refresh() {
    loading.value = true
    try {
      data.value = await apiGet<Overview>(`/api/overview?windowMinutes=${windowMinutes.value}`)
      error.value = null
      lastUpdated.value = Date.now()
    } catch (cause) {
      error.value = cause instanceof ApiError ? cause : new ApiError(0, 'network', String(cause))
    } finally {
      loading.value = false
    }
  }

  function setWindow(minutes: number) {
    windowMinutes.value = minutes
  }

  onMounted(() => {
    void refresh()
    timer = setInterval(refresh, 15_000)
  })
  onUnmounted(() => {
    if (timer) clearInterval(timer)
  })
  watch(windowMinutes, () => void refresh())

  return { windowMinutes, data, error, loading, lastUpdated, refresh, setWindow }
}
