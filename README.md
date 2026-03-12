# gitnexus-rs

中文文档请见 [README.zh-CN.md](./README.zh-CN.md)。

Rust migration baseline for the `gitnexus` package.

This crate is intentionally added **alongside** the existing TypeScript package (`/gitnexus`) so migration can happen incrementally without breaking current workflows.

This repository includes AI-driven migration work and is designed to keep the project runnable in performance-constrained environments (for example, lower-memory or limited-compute local setups) by prioritizing lean native Rust paths.

## License

- This repository is released under `PolyForm Noncommercial License 1.0.0`. See [LICENSE](./LICENSE).
- It contains migrated/derivative work from GitNexus; required attribution is listed in [NOTICE](./NOTICE).
- Commercial use requires separate authorization from the relevant rights holder(s).

## Implemented in Rust (phase 1)

- Native CLI commands: `analyze`, `list`, `status`, `clean`, `setup`, `query`, `context`, `impact`, `cypher`, `detect-changes`, `rename`, `mcp`, `serve`, `wiki`
- Delegated CLI commands (forwarded to the TypeScript CLI for compatibility):
  - `augment`, `eval-server`
- Git helpers:
  - detect git repo
  - resolve git root
  - read current commit
- Storage + registry compatibility with current JSON layout:
  - `.gitnexus/meta.json`
  - `~/.gitnexus/registry.json`

## Implemented in Rust (phase 2 - partial)

- Ingestion pipeline foundation:
  - repository scan with ignore rules
  - large file filtering (`>512KB`)
  - language detection for supported source file extensions
  - structure graph extraction (`Folder`/`File` + `CONTAINS`)
  - heuristic parsing baseline for symbol extraction (`Function`/`Class`/`Interface`/`Struct`/`Enum`/`Trait`)
  - heuristic TypeScript/JavaScript relative import extraction (`IMPORTS`)
  - heuristic TypeScript/JavaScript call extraction (`CALLS`)
  - heuristic TypeScript/JavaScript inheritance extraction (`EXTENDS`/`IMPLEMENTS`)
  - `DEFINES` edges from `File` nodes to extracted symbols
- Ingestion report output:
  - `.gitnexus/ingestion.json`

## Implemented in Rust (phase 3 - bootstrap, partial)

- Local graph runtime bootstrap for native tool commands:
  - native `query`/`context`/`impact`/`cypher`/`detect_changes`/`rename` no longer require TS delegation
  - on-demand graph cache persisted to `.gitnexus/graph.json`
  - fallback behavior: if cache is missing, build from Rust ingestion pipeline and cache locally
- MCP bootstrap:
  - native `mcp` stdio server implemented with Rust MCP SDK (`rmcp`) and `transport::stdio`
  - tool/resource/prompt handling implemented via Rust `ServerHandler` (initialize/tools/resources/prompts/ping)
  - tool execution routes to native Rust command implementations where available
- HTTP serve bootstrap:
  - native `serve` command implemented in Rust (`axum` + tokio listener, default `127.0.0.1:4747`)
  - REST baseline endpoints for repos/graph/query/search/file/processes/clusters
  - `/api/mcp` mounted with `rmcp` Streamable HTTP server transport (`StreamableHttpService`, stateful sessions)
  - session lifecycle delegated to rmcp transport layer with Rust-side transport compatibility normalization
  - removed legacy hand-written TCP/MCP code paths from `local_serve.rs` to avoid dual transport implementations
- Native `cypher` currently supports a safe read-only subset:
  - `MATCH ... RETURN ... [LIMIT n]`
  - node pattern and single-hop relationship pattern
  - `WHERE` with `=` / `CONTAINS` (combined by `AND`)
  - `COUNT(*)` and column projection with `AS`
  - write operations are explicitly blocked
- Kuzu bridge baseline for `query` / `context` / `impact` / `cypher`:
  - primary path: Rust-native Kuzu bridge against local `.gitnexus/kuzu`
  - output compatibility: `query` / `context` / `impact` keep existing JSON schemas; `cypher` keeps `{markdown,row_count}`
  - fallback path: if local Kuzu DB is unavailable, Rust falls back to local `.gitnexus/graph.json` heuristic runtime (`cypher` falls back to the read-only subset evaluator)

## Current behavior

- `analyze` currently performs structure + parsing bootstrap:
  - finds repository root
  - scans repository files using ignore and size rules
  - builds structure graph and file-to-symbol `DEFINES` links
  - adds heuristic `IMPORTS` edges for TypeScript/JavaScript relative imports
  - adds heuristic `CALLS` edges for TypeScript/JavaScript function/class calls
  - adds heuristic `EXTENDS`/`IMPLEMENTS` edges for TypeScript/JavaScript inheritance
  - writes/updates `.gitnexus/meta.json` with file/node/edge stats (`communities` / `processes` are heuristic estimates based on current relation extraction)
  - writes `.gitnexus/ingestion.json` with ingestion stats
  - registers repo in `~/.gitnexus/registry.json`
  - ensures `.gitnexus` is in `.gitignore`
- `query` / `context` / `impact` / `cypher` / `detect-changes` / `rename` now execute via Rust-native command paths.
- `query` / `context` / `impact` / `cypher` now prefer the Rust-native Kuzu bridge when `.gitnexus/kuzu` is present, and automatically fall back to local `.gitnexus/graph.json` heuristics when Kuzu is unavailable.
- `setup` now executes with a Rust-native baseline (global editor MCP config merge, skills installation, Claude hook merge).
- Native `impact` now supports relation filtering parity baseline:
  - `relationTypes` / `relation_types` to filter traversal edges (`CALLS`, `IMPORTS`, `EXTENDS`, `IMPLEMENTS`)
  - `minConfidence` / `min_confidence` to filter by edge confidence threshold (`0..1`)
  - available via both CLI flags (`--relation-types`, `--min-confidence`) and MCP tool arguments
- `mcp` now executes with rmcp-backed Rust-native stdio transport.
- `serve` now executes with a Rust-native HTTP server baseline.
- `/api/mcp` now runs on rmcp Streamable HTTP transport (stateful sessions, `mcp-session-id` lifecycle handled by SDK transport).
- `/api/mcp` transport compatibility hardening (2026-03-12):
  - request header normalization for MCP clients (`Accept`/`Content-Type`) to reduce strict content-negotiation failures
  - SSE priming empty event disabled (`sse_retry: None`) to improve strict client decoder compatibility
  - fallback `Content-Type` is ensured on MCP responses to avoid `Unexpected content type: None` client errors
- `local_serve.rs` no longer keeps the previous custom `/api/mcp` session/SSE/TCP fallback implementation; MCP HTTP is rmcp-only.
- Non-migrated commands are still delegated to the TypeScript implementation when available (local source/build first, then `npx gitnexus@latest` fallback), primarily `augment` and `eval-server`.
- Rust now includes a native `wiki` baseline that generates static markdown docs under `.gitnexus/wiki`.
- Full native Kuzu materialization/loading in `analyze`, FTS, embeddings, and full wiki/LLM parity are not migrated yet.

## Migration progress

Detailed migration progress (including command matrix and parity gaps) has been moved to:

- [MIGRATION_PROGRESS.md](./MIGRATION_PROGRESS.md)
- [MIGRATION_PROGRESS.zh-CN.md](./MIGRATION_PROGRESS.zh-CN.md)

## Why this shape

The TypeScript implementation includes a large multi-phase pipeline (Tree-sitter parsing, graph relationship extraction, Kuzu materialization, embedding generation, MCP/HTTP serving). A full equivalent Rust rewrite should be done in safe stages to keep behavior parity and avoid regressions.

## Suggested phase order

1. Port storage + git + CLI primitives (done)
2. Port ingestion pipeline (structure/parsing/import/call/heritage/community/process)
3. Port graph persistence (Kuzu adapter + schema + loaders)
4. Port search and embeddings
5. Port MCP server and HTTP server
6. Parity tests against existing TS fixtures and integration tests
