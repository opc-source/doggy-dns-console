export interface SeriesPoint {
  ts: number
  queries: number
  failures: number
}

export interface Overview {
  windowMinutes: number
  instancesOnline: number
  instancesTotal: number
  queries: number
  failures: number
  successRate: number | null
  p95LatencyMs: number | null
  series: SeriesPoint[]
}

export interface FailureGroup {
  key: string
  traceId: string | null
  query: string
  qtype: string
  instance: string | null
  authority: string | null
  responseCode: string
  error: string | null
  count: number
  firstSeen: number
  lastSeen: number
  durationMs: number | null
}

export interface LogRecord {
  ts: number
  severity: string
  message: string
  traceId: string | null
  spanId: string | null
  fields: Record<string, unknown>
}

export interface SpanRecord {
  spanId: string
  name: string
  durationMs: number | null
  status: string
  attributes: Record<string, unknown>
}

export interface FailureDetail {
  group: FailureGroup
  logs: LogRecord[]
  spans: SpanRecord[]
}

export interface Instance {
  id: string
  host: string
  version: string | null
  online: boolean
  lastSeen: number
  mcpReachable: boolean
}

export interface PluginInfo {
  order: number
  kind: string
  enabled: boolean
  details: Record<string, unknown>
}

export interface InstanceDetail {
  instance: Instance
  uptimeSeconds: number | null
  startedAt: string | null
  listenAddr: string | null
  protocols: string[]
  plugins: PluginInfo[]
  configToml: string | null
  recentFailures: number
}

export interface Connections {
  openobserve: { ok: boolean; latencyMs: number }
  openobserveMcp: { ok: boolean; latencyMs: number }
  mcp: { reachable: number; total: number }
  model: { configured: boolean }
}

export interface SessionSummary {
  id: string
  appName: string
  userId: string
  lastUpdateTime: number
  events: SessionEvent[]
  state: Record<string, unknown>
}

export interface SessionEvent {
  author?: string
  content?: {
    role?: string
    parts?: Array<{ text?: string }>
  }
}
