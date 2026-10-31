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

export interface RuntimeStatus {
  signals: Record<string, unknown>
}

export interface InstanceDetail {
  instance: Instance
  pid: number | null
  status: string | null
  uptimeSeconds: number | null
  startedAt: string | null
  listenAddr: string | null
  protocols: string[]
  plugins: PluginInfo[]
  configToml: string | null
  redactedKeys: string[]
  runtime: RuntimeStatus | null
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

export type MatchQuality = 'exact' | 'host_ip' | 'host' | 'none'
export type MissingEvidence =
  | 'instance_unreachable'
  | 'instance_not_found'
  | 'spans_unavailable'
  | 'metrics_unavailable'
  | 'logs_truncated'

export interface EvidenceFilter {
  windowMinutes: number
  domain: string | null
  responseCode: string | null
}

export interface EvidenceBundle {
  traceId: string
  filter: EvidenceFilter
  generatedAt: number
  fingerprint: string
  group: FailureGroup
  instance: EvidenceInstance | null
  logs: EvidenceLogs
  spans: EvidenceSpans
  metrics: EvidenceMetrics | null
  missing: MissingEvidence[]
}

export interface EvidenceLogs {
  items: EvidenceLog[]
  total: number
  truncated: boolean
  omitted: number
}

export interface EvidenceLog {
  id: string
  ts: number
  severity: string
  message: string
  spanId: string | null
  query: string | null
  qtype: string | null
  responseCode: string | null
  error: string | null
}

export interface EvidenceSpans {
  items: EvidenceSpan[]
  total: number
  truncated: boolean
}

export interface EvidenceSpan {
  id: string
  spanId: string
  name: string
  durationMs: number | null
  status: string
}

export interface EvidenceInstance {
  matchQuality: MatchQuality
  identity: InstanceIdentity
  reachable: boolean
  status: string | null
  pid: number | null
  uptimeSeconds: number | null
  listenAddr: string | null
  plugins: EvidencePlugin[]
  config: EvidenceConfig | null
  runtime: Record<string, unknown> | null
  missing: MissingEvidence[]
}

export interface InstanceIdentity {
  serviceInstanceId?: string
  host?: string
  ip?: string
}

export interface EvidencePlugin {
  id: string
  order: number
  kind: string
  enabled: boolean
  details: unknown
}

export interface EvidenceConfig {
  id: string
  content: string
  redactedKeys: string[]
}

export interface EvidenceMetrics {
  range: EvidenceRange
  host: EvidenceHostMetrics | null
  global: MetricCounts
  queriesId: string
  failuresId: string
}

export interface EvidenceRange {
  start: number
  end: number
}

export interface EvidenceHostMetrics {
  name: string
  queries: number
  failures: number
  queriesId: string
  failuresId: string
}

export interface MetricCounts {
  queries: number
  failures: number
}

export type AnalysisStatus = 'ok' | 'insufficient_evidence' | 'unavailable' | 'invalid'

export type VerdictCategory =
  | 'upstream_unreachable'
  | 'upstream_timeout'
  | 'upstream_refused'
  | 'policy_refused'
  | 'config_invalid'
  | 'nacos_stale'
  | 'xdp_path'
  | 'unknown'

export type Confidence = 'low' | 'medium' | 'high'
export type FindingKind = 'observed' | 'inferred'
export type RiskLevel = 'low' | 'medium' | 'high'

export interface AnalysisResult {
  status: AnalysisStatus
  verdict: AnalysisVerdict
  findings: Finding[]
  nextSteps: NextStep[]
  notDeterminable: string[]
}

export interface AnalysisVerdict {
  category: VerdictCategory
  summary: string
  confidence: Confidence
}

export interface Finding {
  claim: string
  evidenceIds: string[]
  kind: FindingKind
}

export interface NextStep {
  action: string
  reason: string
  risk: RiskLevel
}

export interface ToolCallAudit {
  name: string
  canonicalArgs: unknown
  durationMs: number
  resultHash: string
  summary: string
}

export type InvestigationStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'invalid'

export interface AnalysisSnapshot {
  analysisId: string
  traceId: string
  status: InvestigationStatus
  attempt: number
  toolCallCount: number
  startedAt: number
  finishedAt: number | null
  filter: EvidenceFilter
  evidenceFingerprint: string
  model: string
  promptVersion: string
  verdictCategory: VerdictCategory | null
  confidence: Confidence | null
  result: AnalysisResult | null
  toolCalls: ToolCallAudit[]
  error: string | null
}

export interface AnalysisAccepted {
  analysisId: string
}

export interface AnalysisMarker {
  id: string
  traceId: string
  status: InvestigationStatus
  verdictCategory: VerdictCategory | null
  confidence: Confidence | null
  attempt: number
  createdAt: number
  finishedAt: number | null
}
