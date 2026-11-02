import { onScopeDispose, ref, getCurrentScope, type Ref } from 'vue'

export function useMediaQuery(query: string): Ref<boolean> {
  const list = window.matchMedia(query)
  const matches = ref(list.matches)
  const handler = (event: MediaQueryListEvent) => {
    matches.value = event.matches
  }
  list.addEventListener('change', handler)
  if (getCurrentScope()) onScopeDispose(() => list.removeEventListener('change', handler))
  return matches
}
