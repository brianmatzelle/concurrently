# concurrently

Concurrent subagent workflow TUI. Each agent is a real Claude Code process (`claude -p`) running in parallel with full tool access — file editing, bash, search, everything.

## Build & Run

```bash
export ANTHROPIC_API_KEY=sk-ant-...
bun install
bun run dev
```

For development with auto-rebuild:
```bash
# Watch for changes and rebuild
bun run build   # then bun run start
```

Requires `claude` CLI installed and on PATH, and Bun runtime.

## Architecture

```
User prompt
    │
    ▼
Orchestrator (raw Anthropic API, non-streaming)
    │  decomposes task into 2-5 subtasks
    ▼
┌────────────────────────────────┐
│  Bun.spawn() × N               │
│  claude -p "task"              │
│    --output-format stream-json │
│    --append-system-prompt ...  │
│    --dangerously-skip-perms    │
│  Each agent = real Claude Code │
└────────────┬───────────────────┘
             │ Direct $state mutation
             ▼
         Svelte 5 reactive rendering
             │
             ▼
         SvelTUI (terminal UI)
```

### Source Files

- `src/main.ts` — Entry point, API key validation, SvelTUI mount (fullscreen)
- `src/App.svelte` — Root component, keyboard handling, mode-based layout switching
- `src/lib/state.svelte.ts` — AppState class with Svelte 5 `$state` runes, agent lifecycle, synthesizer
- `src/lib/agent.ts` — Spawns `claude -p` via `Bun.spawn()`, parses `stream-json` stdout into events
- `src/lib/api.ts` — Raw Anthropic API client (streaming SSE + non-streaming) via native `fetch`
- `src/lib/orchestrator.ts` — Task decomposition: sends user prompt to Claude, gets back JSON array of subtasks
- `src/components/Header.svelte` — Header bar with agent count, kernel size, elapsed time
- `src/components/InputView.svelte` — Help text + task input box
- `src/components/AgentList.svelte` — Left sidebar with agent status icons and info
- `src/components/AgentDetail.svelte` — Right panel with selected agent's streaming output
- `src/components/StatusBar.svelte` — Bottom bar with status message and keybinding hints

### Key Concepts

- **Kernel** (`Message[]`): Shared conversation history. Every agent and the orchestrator see it. When agents complete, results fold back into the kernel. Persists across rounds (press `n` for new task).
- **Reactive state**: No event loop or channels. Agent callbacks directly mutate Svelte 5 `$state`, triggering automatic re-renders. All state lives in the `AppState` class (`state.svelte.ts`).
- **stream-json parsing**: Each `claude -p` outputs newline-delimited JSON. We parse `assistant` messages (extract text deltas + tool_use blocks) and `result` messages (done/error + cost).

## Key Bindings

- **Input mode**: Type task, `Enter` to submit
- **Running mode**: `↑`/`↓` select agent, `j`/`k` scroll output
- **Done mode**: `s` synthesize, `n` new task (keeps kernel), `q` quit
- `Ctrl+C` quits from any mode

## Agent Flags

Each agent spawns with:
```
claude -p <task>
  --append-system-prompt <kernel context>
  --output-format stream-json
  --include-partial-messages
  --dangerously-skip-permissions
  --no-session-persistence
  --model sonnet
```

## Distribution

- **AUR**: `yay -S concurrently-bin` — PKGBUILD lives in `~/projects/maintaining/concurrently-bin/`
- **Homebrew**: `brew tap brianmatzelle/tap && brew install concurrently`
- **GitHub**: `brianmatzelle/concurrently`, releases have Linux x86_64 binaries
