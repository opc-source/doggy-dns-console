import { onMounted, onUnmounted, ref, watch, type Ref } from 'vue'

import { ApiError, apiGet } from '@/api/http'
import type { Overview } from '@/api/types'
import {
  isAbsolute,
  resolveBracket,
  toRequestQuery,
  validateBracket,
  validateDraft,
  type FilterDraft,
} from '@/lib/failureFilter'

const POLL_INTERVAL_MS = 15_000

export function useOverview(draft: Ref<FilterDraft>) {
  const data = ref<Overview | null>(null)
  const error = ref<ApiError | null>(null)
  const invalid = ref<string | null>(null)
  const loading = ref(false)
  const lastUpdated = ref<number | null>(null)
  let timer: ReturnType<typeof setInterval> | null = null
  let seq = 0

  async function refresh() {
    // Bump before looking at the draft: an answer still in flight describes the
    // bracket that was just replaced (or just refused), so it must not land.
    const requestSeq = ++seq
    const now = Date.now()
    const refusal =
      validateDraft(draft.value) ?? validateBracket(resolveBracket(draft.value, now), now)
    if (refusal) {
      // The URL asks something illegal: say so, show nothing stale behind it.
      invalid.value = refusal
      data.value = null
      error.value = null
      return
    }
    invalid.value = null
    loading.value = true
    try {
      const query = new URLSearchParams(toRequestQuery(draft.value)).toString()
      const fetched = await apiGet<Overview>(`/api/overview?${query}`)
      if (requestSeq !== seq) return
      data.value = fetched
      error.value = null
      lastUpdated.value = Date.now()
    } catch (cause) {
      if (requestSeq !== seq) return
      error.value = cause instanceof ApiError ? cause : new ApiError(0, 'network', String(cause))
    } finally {
      if (requestSeq === seq) loading.value = false
    }
  }

  function restartTimer() {
    if (timer) {
      clearInterval(timer)
      timer = null
    }
    // An absolute bracket is a fixed question; refreshing it on a timer only
    // moves the answer under the operator's feet.
    if (!isAbsolute(draft.value)) timer = setInterval(() => void refresh(), POLL_INTERVAL_MS)
  }

  onMounted(() => {
    void refresh()
    restartTimer()
  })
  onUnmounted(() => {
    if (timer) clearInterval(timer)
  })
  watch(draft, () => {
    void refresh()
    restartTimer()
  })

  return { data, error, invalid, loading, lastUpdated, refresh }
}
