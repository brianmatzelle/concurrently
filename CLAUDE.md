# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

Concurrent subagent workflow TUI. Each agent is a real Claude Code process (`claude -p`) running in parallel with full tool access. Built with Svelte 5 + SvelTUI (terminal rendering via yoga-layout).

Branch `pivot/svelTUI` is the active branch — the Rust/ratatui version (`src/*.rs`, `Cargo.toml`) is legacy from `main`.

## Build & Run

```bash
export ANTHROPIC_API_KEY=sk-ant-...
bun install
bun run dev          # build + start
bun run build        # just compile (./sveltui-build)
bun run start        # run compiled output (bun --conditions browser dist/src/main.mjs)
bun run watch        # auto-rebuild on changes (needs watchexec)
```

Requires `claude` CLI on PATH and Bun runtime.

### Post-install Patch (Required)

After `bun install`, patch `node_modules/@rlabs-inc/sveltui/src/mount.svelte.ts` lines 65-67 — remove the three `onKey` calls (`Tab`, `Shift+Tab`, `Escape`). SvelTUI's built-in focus handlers swallow these events before app handlers run. Rebuild after patching.

## Architecture

```
User input → AppState.submitInput()
  → createNewAgent() or respondToAgent()
    → spawnAgent(): Bun.spawn("claude -p ...")
      → stdout stream-json → parseStreamLine() → AgentEvent callbacks
        → direct $state mutation on AppState → Svelte re-render
```

**Four source files (all in `src/`):**

- `main.ts` — Entry point. Validates `ANTHROPIC_API_KEY`, mounts SvelTUI fullscreen, mounts Svelte `App` component.
- `App.svelte` — Single-file UI. Keyboard handling, log display, agent status bar, input line. All rendering logic lives here.
- `lib/state.svelte.ts` — `AppState` singleton with Svelte 5 `$state` runes. Manages agents, log, input, scroll. Exports `app`.
- `lib/agent.ts` — Agent types, `spawnAgent()` (Bun.spawn + stream-json parsing), `createAgent()` factory.

### Key Concepts

- **No orchestrator on this branch.** Users type tasks directly. `"name: task"` syntax names agents; plain text auto-names them `agent-N`.
- **Conversation persistence**: Each agent has a `conversation: Message[]`. When done, users can send follow-up messages to the selected agent — it respawns `claude -p` with prior conversation as system context.
- **Reactive state, no event loop**: Agent stdout callbacks mutate `$state` directly on the `AppState` singleton, triggering Svelte re-renders.
- **stream-json parsing**: `claude -p --output-format stream-json` emits newline-delimited JSON. `parseStreamLine()` extracts `assistant` messages (text deltas + tool_use blocks) and `result` messages (done/error + cost).
- **`/s` prefix**: `/s <task>` forces spawning a new agent even when one is selected.

### Build System

`./sveltui-build` is a custom Bun script that compiles all `.svelte` and `.svelte.ts` files (app + SvelTUI framework) in a single pass via `svelte/compiler`, then fixes import paths in the output. Required because Svelte 5 reactivity needs unified compilation.

## Key Bindings

- Any printable key: appends to input
- `Enter`: submit input (spawn agent or respond to selected agent)
- `Tab`: cycle selected agent
- `Escape`: deselect agent
- `PageUp`/`PageDown`: scroll log
- `Ctrl+C`: quit

## Agent Spawn Flags

```
claude -p <task>
  --append-system-prompt <conversation context>
  --output-format stream-json
  --verbose
  --dangerously-skip-permissions
  --no-session-persistence
  --model sonnet
```

## Distribution

- **AUR**: `yay -S concurrently-bin` — PKGBUILD lives in `~/projects/maintaining/concurrently-bin/`
- **Homebrew**: `brew tap brianmatzelle/tap && brew install concurrently`
- **GitHub**: `brianmatzelle/concurrently`
