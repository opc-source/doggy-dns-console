import { onUnmounted } from 'vue'

export function usePolling(task: () => unknown, intervalMs: number) {
  const timer = setInterval(task, intervalMs)
  onUnmounted(() => clearInterval(timer))
  return () => clearInterval(timer)
}
