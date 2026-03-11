<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **gitnexus-rs** (1862 symbols, 4861 relationships, 141 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> If any GitNexus tool warns the index is stale, run `npx gitnexus analyze` in terminal first.

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.

## When Debugging

1. `gitnexus_query({query: "<error or symptom>"})` — find execution flows related to the issue
2. `gitnexus_context({name: "<suspect function>"})` — see all callers, callees, and process participation
3. `READ gitnexus://repo/gitnexus-rs/process/{processName}` — trace the full execution flow step by step
4. For regressions: `gitnexus_detect_changes({scope: "compare", base_ref: "main"})` — see what your branch changed

## When Refactoring

- **Renaming**: MUST use `gitnexus_rename({symbol_name: "old", new_name: "new", dry_run: true})` first. Review the preview — graph edits are safe, text_search edits need manual review. Then run with `dry_run: false`.
- **Extracting/Splitting**: MUST run `gitnexus_context({name: "target"})` to see all incoming/outgoing refs, then `gitnexus_impact({target: "target", direction: "upstream"})` to find all external callers before moving code.
- After any refactor: run `gitnexus_detect_changes({scope: "all"})` to verify only expected files changed.

## Never Do

- NEVER edit a function, class, or method without first running `gitnexus_impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `gitnexus_rename` which understands the call graph.
- NEVER commit changes without running `gitnexus_detect_changes()` to check affected scope.

## Tools Quick Reference

| Tool | When to use | Command |
|------|-------------|---------|
| `query` | Find code by concept | `gitnexus_query({query: "auth validation"})` |
| `context` | 360-degree view of one symbol | `gitnexus_context({name: "validateUser"})` |
| `impact` | Blast radius before editing | `gitnexus_impact({target: "X", direction: "upstream"})` |
| `detect_changes` | Pre-commit scope check | `gitnexus_detect_changes({scope: "staged"})` |
| `rename` | Safe multi-file rename | `gitnexus_rename({symbol_name: "old", new_name: "new", dry_run: true})` |
| `cypher` | Custom graph queries | `gitnexus_cypher({query: "MATCH ..."})` |

## Impact Risk Levels

| Depth | Meaning | Action |
|-------|---------|--------|
| d=1 | WILL BREAK — direct callers/importers | MUST update these |
| d=2 | LIKELY AFFECTED — indirect deps | Should test |
| d=3 | MAY NEED TESTING — transitive | Test if critical path |

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/gitnexus-rs/context` | Codebase overview, check index freshness |
| `gitnexus://repo/gitnexus-rs/clusters` | All functional areas |
| `gitnexus://repo/gitnexus-rs/processes` | All execution flows |
| `gitnexus://repo/gitnexus-rs/process/{name}` | Step-by-step execution trace |

## Self-Check Before Finishing

Before completing any code modification task, verify:
1. `gitnexus_impact` was run for all modified symbols
2. No HIGH/CRITICAL risk warnings were ignored
3. `gitnexus_detect_changes()` confirms changes match expected scope
4. All d=1 (WILL BREAK) dependents were updated

## Keeping the Index Fresh

After committing code changes, the GitNexus index becomes stale. Re-run analyze to update it:

```bash
npx gitnexus analyze
```

If the index previously included embeddings, preserve them by adding `--embeddings`:

```bash
npx gitnexus analyze --embeddings
```

To check whether embeddings exist, inspect `.gitnexus/meta.json` — the `stats.embeddings` field shows the count (0 means no embeddings). **Running analyze without `--embeddings` will delete any previously generated embeddings.**

> Claude Code users: A PostToolUse hook handles this automatically after `git commit` and `git merge`.

## CLI

- Re-index: `npx gitnexus analyze`
- Check freshness: `npx gitnexus status`
- Generate docs: `npx gitnexus wiki`

<!-- gitnexus:end -->

## Skills
A skill is a set of local instructions stored in a `SKILL.md` file. The list below reflects currently installed skills under `.agents/skills`.

### Available skills
- coding-guidelines: Use when asking about Rust code style or best practices. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/coding-guidelines/SKILL.md)
- domain-cli: Use when building CLI tools. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/domain-cli/SKILL.md)
- domain-cloud-native: Use when building cloud-native apps. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/domain-cloud-native/SKILL.md)
- domain-embedded: Use when developing embedded/no_std Rust. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/domain-embedded/SKILL.md)
- domain-fintech: Use when building fintech apps. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/domain-fintech/SKILL.md)
- domain-iot: Use when building IoT apps. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/domain-iot/SKILL.md)
- domain-ml: Use when building ML/AI apps in Rust. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/domain-ml/SKILL.md)
- domain-web: Use when building web services. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/domain-web/SKILL.md)
- m01-ownership: CRITICAL: Use for ownership/borrow/lifetime issues. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m01-ownership/SKILL.md)
- m02-resource: CRITICAL: Use for smart pointers and resource management. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m02-resource/SKILL.md)
- m03-mutability: CRITICAL: Use for mutability issues. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m03-mutability/SKILL.md)
- m04-zero-cost: CRITICAL: Use for generics, traits, zero-cost abstraction. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m04-zero-cost/SKILL.md)
- m05-type-driven: CRITICAL: Use for type-driven design. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m05-type-driven/SKILL.md)
- m06-error-handling: CRITICAL: Use for error handling. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m06-error-handling/SKILL.md)
- m07-concurrency: CRITICAL: Use for concurrency/async. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m07-concurrency/SKILL.md)
- m09-domain: CRITICAL: Use for domain modeling. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m09-domain/SKILL.md)
- m10-performance: CRITICAL: Use for performance optimization. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m10-performance/SKILL.md)
- m11-ecosystem: Use when integrating crates or ecosystem questions. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m11-ecosystem/SKILL.md)
- m12-lifecycle: Use when designing resource lifecycles. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m12-lifecycle/SKILL.md)
- m13-domain-error: Use when designing domain error handling. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m13-domain-error/SKILL.md)
- m14-mental-model: Use when learning Rust concepts. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m14-mental-model/SKILL.md)
- m15-anti-pattern: Use when reviewing code for anti-patterns. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/m15-anti-pattern/SKILL.md)
- meta-cognition-parallel: EXPERIMENTAL: Three-layer parallel meta-cognition analysis. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/meta-cognition-parallel/SKILL.md)
- rust-call-graph: Visualize Rust function call graphs using LSP. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/rust-call-graph/SKILL.md)
- rust-code-navigator: Navigate Rust code using LSP. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/rust-code-navigator/SKILL.md)
- rust-daily: CRITICAL: Use for Rust news and daily/weekly/monthly reports. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/rust-daily/SKILL.md)
- rust-deps-visualizer: Visualize Rust project dependencies as ASCII art. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/rust-deps-visualizer/SKILL.md)
- rust-learner: Use when asking about Rust versions or crate info. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/rust-learner/SKILL.md)
- rust-refactor-helper: Safe Rust refactoring with LSP analysis. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/rust-refactor-helper/SKILL.md)
- rust-router: CRITICAL: Use for ALL Rust questions including errors, design, and coding. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/rust-router/SKILL.md)
- rust-skill-creator: Use when creating skills for Rust crates or std library documentation. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/rust-skill-creator/SKILL.md)
- rust-symbol-analyzer: Analyze Rust project structure using LSP symbols. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/rust-symbol-analyzer/SKILL.md)
- rust-trait-explorer: Explore Rust trait implementations using LSP. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/rust-trait-explorer/SKILL.md)
- unsafe-checker: CRITICAL: Use for unsafe Rust code review and FFI. (file: /home/ymx/life2/gitnexus-rs/.agents/skills/unsafe-checker/SKILL.md)

### How to use skills
- Trigger rules: If a user names a skill (for example, `$skill-name` or plain text) or the task clearly matches a skill description, use that skill for the turn.
- Read progressively: Open `SKILL.md`, then only load extra referenced files when needed.
- Path resolution: Resolve relative paths in a skill relative to that skill's folder first.
- Keep context focused: Do not bulk-load all skill assets; read only what is needed for the current request.
- Fallback: If a skill file is missing or not readable, report it briefly and continue with the best available approach.
