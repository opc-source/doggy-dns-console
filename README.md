# Doggy DNS Console

An AI-first, read-only operations console for [doggy-dns](../doggy-dns). The dashboard reads
OpenObserve REST; the copilot uses OpenObserve MCP (allowlisted read-only tools) and the
doggy-dns instance MCP. No login: bind it to loopback by default.

## Quick start (no external services)

```bash
# terminal 1
CONSOLE_DEMO=1 cargo run

# terminal 2
cd doggy-dns-ui && bun install && bun run dev
# open http://localhost:5173
```

`CONSOLE_DEMO=1` uses built-in fixtures and a scripted model, so the overview, failure drawer,
and one AI turn work without OpenObserve, doggy-dns, or an API key. The production build can be
served by the Rust binary:

```bash
cd doggy-dns-ui && bun run build
cd .. && CONSOLE_DEMO=1 cargo run
# open http://127.0.0.1:8787
```

## Real deployment

1. Copy `.env.example` to `.env` and fill it in.
2. Set `OPENOBSERVE_BASE_URL`, `OPENOBSERVE_ORG`, and either `OPENOBSERVE_USER`/`OPENOBSERVE_PASSWORD` or `OPENOBSERVE_TOKEN`.
3. Set `MODEL_*` for any OpenAI-compatible endpoint.
4. Set `MCP_ENDPOINTS` to the doggy-dns MCP URLs (for example
   `http://127.0.0.1:19090/mcp`).
5. `cargo run`.

OpenObserve MCP is optional but recommended. If `/api/{org}/mcp` returns 404, enable it on the
OpenObserve instance (`O2_AI_ENABLED=true`, and `O2_TOOL_API_URL` where required). The console
only registers an allowlist: `StreamList`, `StreamSchema`, `SearchSQL`, `PrometheusQuery`,
`PrometheusRangeQuery`, `GetLatestTraces`, `GetTraceDAG`, `SearchAround`. It never exposes
`tool_search`, `tools_call`, or any write tool to the model. OpenObserve OSS has no granular
RBAC for MCP, so use a dedicated account and network restrictions.

Run the probes after upgrading OpenObserve:

```bash
cargo run --bin openobserve_probe       # REST streams, sample logs and metrics
cargo run --bin openobserve_mcp_probe  # MCP tools and allowlisted schemas
```

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

cd doggy-dns-ui
bun run type-check
bun run test:unit -- --run
bun run build
bun run test:e2e
```

The frontend dev server proxies `/api` to `http://127.0.0.1:8787`. Playwright starts the demo
backend and the Vite dev server automatically.

## Boundaries

- Read-only: no config writes, no DNS mutations, no alert management.
- SQLite stores only agent sessions (`data/console.db`); telemetry is not copied locally.
- Success rate excludes `NXDOMAIN`; a window with zero queries shows `—`.
- `NXDOMAIN` detail rows are not exported by doggy-dns (they are `info!`); aggregate counts
  still come from metrics.
