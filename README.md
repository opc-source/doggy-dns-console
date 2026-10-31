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
3. Set `MODEL_*` for any OpenAI-compatible endpoint. `MODEL_BASE_URL` falls back to
   `OPENAI_BASE_URL`, and `MODEL_API_KEY` falls back to `QWEN_API_KEY` then `OPENAI_API_KEY`.
4. Instances are discovered automatically from OpenObserve telemetry: doggy-dns
   exports `host.name`, `doggy_dns.ip_addr` and `doggy_dns.control.port`, which
   the console joins into `http://{ip_addr}:{control_port}/mcp`. Nodes that
   publish no address are probed at `http://{host}:{MCP_CONTROL_PORT}/mcp`
   (default `19090`). `doggy_dns.ip_addr` is the DNS listener's reachable
   address; if the control plane binds a different interface, list that node in
   `MCP_ENDPOINTS` instead. Set `MCP_TOKEN` to the control-plane token, if set.
5. Optional: set `MCP_ENDPOINTS` (comma separated) to probe extra or
   misconfigured nodes that never reach OpenObserve.
6. `cargo run`.

The console consumes all four read-only tools from each instance's control plane: `get_status`,
`list_plugins`, `get_config` and `get_runtime_status`. The instance drawer shows pid/status,
plugins, the redacted TOML (with the masked keys listed) and live signals such as Nacos cache
size/push freshness and XDP attachment. `MCP_TOKEN` is sent as a bearer token to every probed
endpoint — static and discovered alike — so anyone able to publish metrics for this OpenObserve
org can make the console present that token to an address of their choosing; keep the metric
stream trusted or leave `MCP_TOKEN` unset. Per-node tokens are not supported yet.

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
- `NXDOMAIN` is a normal negative answer, never a failure: the failure list and failure
  detail always exclude it, even when `responseCode=NXDOMAIN` is requested.
