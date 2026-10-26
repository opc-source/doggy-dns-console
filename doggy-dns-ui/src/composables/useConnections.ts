import { computed, onMounted, onUnmounted, ref } from 'vue'

import { apiGet } from '@/api/http'
import type { Connections } from '@/api/types'

export function useConnections() {
  const data = ref<Connections | null>(null)
  const failed = ref(false)
  let timer: ReturnType<typeof setInterval> | null = null

  async function refresh() {
    try {
      data.value = await apiGet<Connections>('/api/connections')
      failed.value = false
    } catch {
      failed.value = true
    }
  }

  onMounted(() => {
    void refresh()
    timer = setInterval(refresh, 30_000)
  })
  onUnmounted(() => {
    if (timer) clearInterval(timer)
  })

  const telemetryDown = computed(() => failed.value || (data.value ? !data.value.openobserve.ok : false))
  const rawQueryDown = computed(
    () => failed.value || (data.value ? !data.value.openobserveMcp.ok : false),
  )
  return { data, telemetryDown, rawQueryDown, refresh }
}
