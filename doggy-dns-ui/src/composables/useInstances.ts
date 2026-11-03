import { onMounted, onUnmounted, ref, watch, type Ref } from 'vue'

import { apiGet } from '@/api/http'
import type { Instance, InstanceDetail } from '@/api/types'
import {
  resolveBracket,
  toRequestQuery,
  validateBracket,
  validateDraft,
  type FilterDraft,
} from '@/lib/failureFilter'

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
  const invalid = ref<string | null>(null)
  const loading = ref(false)
  let seq = 0

  async function refresh() {
    // Bump before doing anything else: an answer still in flight describes the
    // bracket that was just replaced (or the drawer that was just closed), so it
    // must not land over the newer one — the same `++seq` retirement
    // `useOverview` and the three counters in `useFailureList` already do.
    const requestSeq = ++seq
    if (!id.value) {
      // Closing the drawer clears the whole panel state, not just `detail`: a
      // `detailError` left set here would survive into the next open and render
      // as a stale failure over an unrelated host.
      detail.value = null
      error.value = null
      invalid.value = null
      return
    }
    // Refuse a hand-edited URL that names a bracket the request cannot honour
    // (a lone `from`/`to`, or one outside 5–10080 minutes) exactly the way
    // `useOverview` refuses it: report locally, never send the empty query the
    // old `toRequestQuery` produced and let the server's default hour answer for.
    const now = Date.now()
    const refusal =
      validateDraft(draft.value) ?? validateBracket(resolveBracket(draft.value, now), now)
    if (refusal) {
      invalid.value = refusal
      detail.value = null
      error.value = null
      loading.value = false
      return
    }
    invalid.value = null
    loading.value = true
    try {
      // The bracket travels with the request: the failure number is a range
      // answer, everything else in the detail is live MCP data.
      const query = new URLSearchParams(toRequestQuery(draft.value)).toString()
      const fetched = await apiGet<InstanceDetail>(
        `/api/instances/${encodeURIComponent(id.value)}?${query}`,
      )
      if (requestSeq !== seq) return
      detail.value = fetched
      error.value = null
    } catch (cause) {
      if (requestSeq !== seq) return
      error.value = cause instanceof Error ? cause.message : String(cause)
    } finally {
      // Only the newest request owns the spinner; a retired one must not clear it.
      if (requestSeq === seq) loading.value = false
    }
  }

  // `immediate` rather than `onMounted`: this composable owns no timer, so it
  // needs no lifecycle hook — and that is also why its spec can call it without
  // a host component, unlike `useOverview`.
  watch([id, draft], () => void refresh(), { immediate: true })
  return { detail, error, invalid, loading, refresh }
}
