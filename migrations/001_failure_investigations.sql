CREATE TABLE IF NOT EXISTS failure_investigations (
  id                   TEXT PRIMARY KEY,
  trace_id             TEXT NOT NULL,
  filter_json          TEXT NOT NULL,
  evidence_fingerprint TEXT NOT NULL,
  model                TEXT NOT NULL,
  prompt_version       TEXT NOT NULL,
  status               TEXT NOT NULL,
  verdict_category     TEXT,
  confidence           TEXT,
  result_json          TEXT,
  tool_calls_json      TEXT,
  error                TEXT,
  attempt              INTEGER NOT NULL,
  created_at           INTEGER NOT NULL,
  finished_at          INTEGER
);
CREATE INDEX IF NOT EXISTS idx_investigations_trace
  ON failure_investigations(trace_id, evidence_fingerprint, status);
CREATE INDEX IF NOT EXISTS idx_investigations_created
  ON failure_investigations(created_at);
