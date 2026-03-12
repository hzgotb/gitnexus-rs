# Migration Progress (TS Baseline)

中文版请见 [MIGRATION_PROGRESS.zh-CN.md](./MIGRATION_PROGRESS.zh-CN.md)。

Last updated: `2026-03-12`

## Scope and Baseline

This document is a TS-baseline audit:

- Baseline (target): `GitNexus/gitnexus/src` (TypeScript implementation)
- Current Rust implementation: `src`
- Evidence source: GitNexus MCP (`query/context/cypher`) + direct code inspection

### MCP Evidence Snapshot (2026-03-12)

- TS ingestion module size: `20` files (`GitNexus/gitnexus/src/core/ingestion/**`)
- Rust ingestion module size: `1` file (`src/ingestion/mod.rs`)
- TS embeddings module size: `5` files (`.../core/embeddings/**`)
- TS search module size: `2` files (`.../core/search/**`)
- TS wiki module size: `5` files (`.../core/wiki/**`)
- TS MCP module size: `8` files (`.../src/mcp/**`)
- TS server module size: `2` files (`.../src/server/**`)

## Current Rust-Only Status (TS as the reference line)

### Command-level status

- Rust-only (or Rust-first command path):  
  `setup`, `analyze`, `list`, `status`, `clean`, `wiki`, `query`, `context`, `impact`, `cypher`, `detect-changes`, `rename`, `mcp`, `serve`
- Not Rust-only (still TS delegated):  
  `augment`, `eval-server`

Note:

- `query/context/impact/cypher/detect-changes/rename/mcp/serve` currently route through `run_ts_cli(...)` wrappers, but are intercepted by Rust native dispatch (`local_tools::try_run_native_tool`) first.
- For query-like tools, Rust path is Kuzu-bridge first (`local_kuzu`) with heuristic fallback, not full TS parity.

### Capability-level status (more important than command names)

- TS `analyzeCommand` executes full staged pipeline including:
  - ingestion pipeline orchestration (`runPipelineFromRepo`)
  - Kuzu load/materialization (`loadGraphToKuzu`)
  - FTS build (`createFTSIndex`)
  - embeddings pipeline (`runEmbeddingPipeline`, cache reuse path)
- Rust `analyze::run` currently does:
  - `run_ingestion_pipeline`
  - save meta/report/register
  - prints `--embeddings` not implemented
- TS `wikiCommand` + `WikiGenerator` includes:
  - LLM config resolution, interactive setup, richer generation workflow, optional gist publish
- Rust `wiki::run` is a baseline static generator and explicitly marks `model/base-url/api-key/concurrency/gist` as not implemented

## Phase Plan (TS baseline, list only NOT Rust-only content)

The following follows the project migration rule sequence (Phase 1 -> Phase 6).

### Phase 1: Storage + Git + CLI Primitives

Goal: Remove remaining TS runtime dependency at CLI primitive level.

Remaining NOT Rust-only items:

1. `augment` command is TS delegated (`src/commands/augment.rs` -> `run_ts_cli("augment", ...)`).
2. `eval-server` command is TS delegated (`src/commands/eval_server.rs` -> `run_ts_cli("eval-server", ...)`).

Steps:

1. Implement Rust-native `augment` fast-path (hook-safe, stderr output contract, graceful failure behavior aligned with TS).
2. Implement Rust-native `eval-server` (LLM-friendly formatter output parity for `query/context/impact/cypher/detect_changes/rename`).
3. Keep TS fallback behind explicit feature flag (instead of default runtime dependency).

### Phase 2: Ingestion Pipeline Parity

Goal: Move from heuristic single-file pipeline to TS-equivalent ingestion depth.

Remaining NOT Rust-only items:

1. Tree-sitter parser-loader capability parity (language availability and parser management).
2. Chunked parse pipeline + worker pool style processing.
3. Import resolution context parity (tsconfig paths / go module / composer / csproj style resolution paths from TS side).
4. Route extraction and richer call/heritage extraction parity.
5. Community/process processors parity (quality and metadata depth).

Steps:

1. Split `src/ingestion/mod.rs` into processors aligned with TS pipeline stages.
2. Introduce parser-loader abstraction and language capability matrix.
3. Add chunked parsing with bounded memory budget and progress callbacks.
4. Port advanced import/call/heritage processing and route extraction.
5. Port community/process detection with deterministic output schema.

### Phase 3: Graph Persistence (Kuzu + Schema + Loaders)

Goal: Make Rust `analyze` produce TS-grade query-ready graph artifacts.

Remaining NOT Rust-only items:

1. Analyze-time Kuzu materialization parity is missing.
2. Analyze-time FTS build parity is missing.
3. Robust incremental reload/update behavior parity is missing.

Steps:

1. Move Kuzu load path from query-time fallback model into analyze pipeline.
2. Align Kuzu schema and loader behavior to TS baseline.
3. Implement FTS index build/update in Rust analyze stage.
4. Add integrity checks and safe rebuild strategy for lock/wal/reset paths.

### Phase 4: Search + Embeddings

Goal: Reach TS retrieval depth and semantic quality.

Remaining NOT Rust-only items:

1. Embedding generation pipeline (including cache reuse) is missing in Rust.
2. Hybrid retrieval parity (lexical + semantic fusion) is incomplete.
3. `/api/search` behavior parity is incomplete.

Steps:

1. Implement embedding pipeline and storage format parity.
2. Add hybrid ranking strategy with reproducible scoring.
3. Align API output schema and ranking semantics with TS.
4. Add recall/precision regression tests for representative queries.

### Phase 5: MCP + HTTP Serving

Goal: Full runtime parity for tool/resource/prompt/server behavior.

Remaining NOT Rust-only items:

1. MCP resources/prompts/setup guidance parity is still simplified compared to TS.
2. Eval-focused serving path (`eval-server`) is not Rust-native yet.
3. `augment` hook workflow is not Rust-native yet.

Steps:

1. Complete MCP resource/prompt parity (including setup and staleness hints).
2. Port eval-server endpoint contract and formatter behavior.
3. Port augment engine behavior used by hook workflow.
4. Add compatibility tests for Cursor/Claude/OpenCode integrations.

### Phase 6: Parity Validation and Rollout

Goal: Prevent silent drift from TS behavior after migration.

Remaining NOT Rust-only items:

1. Systematic parity test matrix is incomplete.
2. Golden outputs across major commands are incomplete.
3. Performance/memory guardrails across large repos are incomplete.

Steps:

1. Build command-level golden tests (`analyze/query/context/impact/cypher/wiki/serve/mcp`).
2. Build integration fixtures for known tricky repos/languages.
3. Define SLOs (latency/memory/index size) and gate in CI.
4. Add phased cutover plan: Rust default -> TS fallback optional -> TS removal.

## Recently Updated Functionality (vs older snapshot)

1. Rust native tool runtime path exists for `query/context/impact/cypher/detect-changes/rename/mcp/serve` through local tool dispatcher.
2. Rust query/context/impact/cypher have Kuzu bridge path with heuristic fallback path.
3. Rust `impact` already exposes relation filters and confidence threshold controls (`relation_types`, `min_confidence`) in CLI/MCP surface.

## Better Suggestions (for faster and safer migration)

1. Remove ambiguity in command ownership: replace `run_ts_cli` wrappers with explicit Rust command entrypoints where Rust is already the default path.
2. Prioritize Phase 1 completion (`augment`, `eval-server`) before deepening parity elsewhere to eliminate last hard TS runtime dependency.
3. Treat `analyze` as the critical path: without analyze-time Kuzu/FTS/embeddings parity, advanced Rust tools will keep diverging from TS quality.
4. Add a parity dashboard in repo (phase -> command -> capability -> test status) and enforce updates in PR checklist.
