# Current Operational State

- **Active Milestone:** v2.5.2 review, diagnostics, and core hardening
- **Release:** v2.5.2 - seven root-cause fixes: `review.rs` now creates dangling checkpoints through temporary Git indexes without moving branches or touching the real index, compares worktree trees on unborn branches, and parses unified-diff hunks, `/dev/null` deletions, offsets, counts, and header-like content correctly; `diagnostics.rs` preserves Windows drive-letter paths and message colons; `runtime.rs` requires a directory boundary before portable-path tokenization; `knowledge.rs` fills every missing role with its role-best model; `bootstrap.rs` propagates template reset write failures; empty-worktree deletions remain visible to review.
- **Current Assignment:** Complete - v2.5.2 hardening finalized for release.
- **Planning Status:** Complete.
- **Implementation Status:**
  - `va-core`: parser, single-writer task lock, allocator, model knowledge,
    real OpenAI-compatible chat-completions client with Vibe-Link headers,
    telemetry (cost, latency, cache, tok/s, backend), per-model
    `reasoning_effort` coercion, `local_node` (Vibe Local probe, LAN UDP
    discovery, token auth headers), hardware-aware runtime, bootstrap, and
    `modes.rs` (four modes, six language lenses, `ReasoningEffort` ladder,
    accents, contracts, guardrails, verification commands, manifest/extension
    detection).
  - `va-harness`: immutable session DAG with JSONL persistence, automatic
    compaction at 75% of the model window, think-trace eviction, head/tail
    observation slicing, pinned working-set tail, hash-anchored patcher with
    repeated-anchor rejection, RepoMap, diagnostic pinpointer, review, judge
    with language/mode guardrails, sandbox, tournament, Harness^2 surfaces,
    contrastive recovery, `.ai/EXPERIENCE.md`, the execution engine with
    read-only/stagnation enforcement, and the production live planner
    (Boss -> Code -> Verify -> Debug -> Judge).
  - `va-cli`: `vibe-assembly` with `status`, `models`, `doctor`, `init`,
    `reset`, `up`, `repomap`, `diagnose`, `run`, `prompt`, `chat`, `tui`/`top`,
    `profile`, `session`, `mcp`, `patch`, `blocks`, `fork`, `rollback`, and
    `compact`. `init` and `run --auto-combo` auto-seed `.ai/model_combo.txt`.
    `run` and `prompt` accept `--effort auto|low|medium|high|xhigh` and
    `harness inspect` shows the c1..c8 surfaces and lessons. `up` probes and
    spawns Vibe Router and Vibe Local and reports LAN Pocket Workers (human
    and `--json`); `doctor` checks the local inference node; `models` and
    auto-combo merge zero-cost local models when the router is offline.
  - `va-desktop`: native GPUI 0.2.2 Liquid Glass default surface (persisted
    night/day palettes, hover nav with pinned Session/Changes/Editor/History
    drawer, fit-to-height sections, nav-clear card text, real command search,
    circular context ring, attachment cards, merged router/Vibe Local/LAN
    model catalogue with backend badges and $0/local blurbs, direct local
    chat routing when the router is offline, and tok/s + backend + $0.00
    telemetry in the Session drawer and status strip) plus the retokenized
    Paper & Ink
    workbench (ADR-019 layout) with real
    RepoMap/session-DAG/diagnostics/harness data, an 8-character block-hash
    editor and git diff view, an interactive assistant composer, a floating
    command palette, a real context-window meter, a shared native `ui_kit`,
    embedded brand/Windows icon assets, and preview-only design samples.
- **Verification Status:** 298 tests passing, 0 failed; clippy clean with
  `-D warnings`; formatting clean; release build produced both binaries.
- **Resume Anchor:** n/a - wave verified and complete.
- **Blockers:** None.
