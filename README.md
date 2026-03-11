# gitnexus-rs

Rust migration baseline for the `gitnexus` package.

This crate is intentionally added **alongside** the existing TypeScript package (`/gitnexus`) so migration can happen incrementally without breaking current workflows.

This repository includes AI-driven migration work and is designed to keep the project runnable in performance-constrained environments (for example, lower-memory or limited-compute local setups) by prioritizing lean native Rust paths.

## License

- This repository is released under `PolyForm Noncommercial License 1.0.0`. See [LICENSE](./LICENSE).
- It contains migrated/derivative work from GitNexus; required attribution is listed in [NOTICE](./NOTICE).
- Commercial use requires separate authorization from the relevant rights holder(s).

## Implemented in Rust (phase 1)

- Native CLI commands: `analyze`, `list`, `status`, `clean`, `query`, `context`, `impact`, `cypher`, `detect-changes`, `rename`, `mcp`, `serve`
- Delegated CLI commands (forwarded to the TypeScript CLI for compatibility):
  - `setup`, `wiki`
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
  - session lifecycle and SSE priming behavior delegated to rmcp transport layer
  - removed legacy hand-written TCP/MCP code paths from `local_serve.rs` to avoid dual transport implementations
- Native `cypher` currently supports a safe read-only subset:
  - `MATCH ... RETURN ... [LIMIT n]`
  - node pattern and single-hop relationship pattern
  - `WHERE` with `=` / `CONTAINS` (combined by `AND`)
  - `COUNT(*)` and column projection with `AS`
  - write operations are explicitly blocked

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
- `query` / `context` / `impact` / `cypher` / `detect-changes` / `rename` now execute with Rust-native local graph runtime (`.gitnexus/graph.json` cache).
- `mcp` now executes with rmcp-backed Rust-native stdio transport.
- `serve` now executes with a Rust-native HTTP server baseline.
- `/api/mcp` now runs on rmcp Streamable HTTP transport (stateful sessions, SSE priming, `mcp-session-id` lifecycle handled by SDK transport).
- `local_serve.rs` no longer keeps the previous custom `/api/mcp` session/SSE/TCP fallback implementation; MCP HTTP is rmcp-only.
- Non-migrated commands are still delegated to the TypeScript implementation when available (local source/build first, then `npx gitnexus@latest` fallback).
- Graph ingestion parity, Kuzu loading, FTS, embeddings, and wiki generation are not migrated yet.

## Migration status snapshot (2026-03-11)

Based on a side-by-side comparison between Rust (`/src`) and TypeScript (`/GitNexus/gitnexus/src`) implementations:

- Overall migration progress (feature parity vs TypeScript): **~35% to 45%**
- Phase status:
  - **Phase 1**: mostly complete
  - **Phase 2**: partial (usable ingestion baseline, but not parity)
  - **Phase 3**: bootstrap started (local graph runtime + native tool command baseline)
  - **Phase 4-6**: not migrated yet

### Command migration matrix

- Native in Rust:
  - `analyze`, `list`, `status`, `clean`
  - `query`, `context`, `impact`, `cypher`, `detect-changes`, `rename`, `mcp`, `serve` (heuristic/local runtime baseline)
- Delegated to TypeScript:
  - `setup`, `wiki`
  - `augment`, `eval-server`

### What is already in Rust (confirmed)

- Git + storage primitives:
  - Git repo detection/root/commit lookup
  - `.gitnexus/meta.json` and `~/.gitnexus/registry.json` compatibility
- Ingestion baseline:
  - repository scan with ignore and file-size rules
  - symbol extraction heuristics
  - `DEFINES`, `IMPORTS`, `CALLS`, `EXTENDS`, `IMPLEMENTS` heuristic edges
  - heuristic community/process summaries
  - ingestion report output (`.gitnexus/ingestion.json`)

### Largest current gap vs TypeScript

- Rust runtime is now available for core tool commands, but it is heuristic and does **not** yet match TS/Kuzu feature depth.
- The TypeScript implementation already provides:
  - full Tree-sitter based ingestion pipeline
  - Kuzu graph materialization + schema loading
  - full-text search (FTS) + embeddings
  - full Express + StreamableHTTP MCP transport behavior
  - wiki generation
  - advanced graph tools (e.g. `detect_changes`, `rename`) backed by local graph queries

### English executive summary

- Migration parity vs TypeScript is currently around **35% to 45%**.
- Rust has a stable Phase 1 baseline (`analyze/list/status/clean`, git/storage/registry compatibility).
- Rust now includes native baseline implementations of `query/context/impact/cypher/detect-changes/rename/mcp/serve`.
- Phase 2/3 capabilities are usable but still heuristic and partial.
- The biggest missing Rust pieces are Kuzu graph persistence, FTS, embeddings, and wiki generation.
- Practical implication: command surface is more native than before, but advanced graph intelligence is still TS/Kuzu-backed.

## Why this shape

The TypeScript implementation includes a large multi-phase pipeline (Tree-sitter parsing, graph relationship extraction, Kuzu materialization, embedding generation, MCP/HTTP serving). A full equivalent Rust rewrite should be done in safe stages to keep behavior parity and avoid regressions.

## Suggested phase order

1. Port storage + git + CLI primitives (done)
2. Port ingestion pipeline (structure/parsing/import/call/heritage/community/process)
3. Port graph persistence (Kuzu adapter + schema + loaders)
4. Port search and embeddings
5. Port MCP server and HTTP server
6. Parity tests against existing TS fixtures and integration tests
