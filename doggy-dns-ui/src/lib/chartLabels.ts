/**
 * An x-axis tick. A one-hour view has always read `HH:mm`; a week of the same
 * labels would be seven identical columns, so anything spanning more than a day
 * gets the date too. The threshold is about the labels, not the bucket width.
 */
export function seriesTick(ts: number, spanMs: number): string {
  const date = new Date(ts)
  const hour = date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  if (spanMs <= 24 * 60 * 60 * 1000) return hour
  return `${date.toLocaleDateString([], { month: '2-digit', day: '2-digit' })} ${hour}`
}

/**
 * The span the points cover, in milliseconds. Fewer than two points is `0`,
 * which the one-day threshold above reads as "short", so a single point keeps
 * the hour-only label instead of growing a date it cannot contrast with.
 */
export function seriesSpanMs(points: { ts: number }[]): number {
  if (points.length < 2) return 0
  return points[points.length - 1]!.ts - points[0]!.ts
}
