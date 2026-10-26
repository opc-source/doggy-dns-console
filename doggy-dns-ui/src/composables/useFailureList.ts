import { onMounted, onUnmounted, ref, watch, type Ref } from 'vue'

import { apiGet } from '@/api/http'
import type { FailureGroup } from '@/api/types'

export function useFailureList(windowMinutes: Ref<number>, limit = 5) {
  const groups = ref<FailureGroup[]>([])
  const error = ref<string | null>(null)
  let timer: ReturnType<typeof setInterval> | null = null

  async function refresh() {
    try {
      groups.value = await apiGet<FailureGroup[]>(
        `/api/failures?windowMinutes=${windowMinutes.value}&limit=${limit}`,
      )
      error.value = null
    } catch (cause) {
      error.value = String(cause)
    }
  }

  onMounted(() => {
    void refresh()
    timer = setInterval(refresh, 15_000)
  })
  onUnmounted(() => {
    if (timer) clearInterval(timer)
  })
  watch(windowMinutes, () => void refresh())

  return { groups, error, refresh }
}
