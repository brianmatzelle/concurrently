# concurrently

Concurrent subagent workflow TUI built with Rust + ratatui. Like Claude Code, but agents run in parallel instead of sequentially.

## Architecture

- `src/main.rs` - Entry point, terminal setup, event loop (~30fps)
- `src/app.rs` - Application state, agent lifecycle management
- `src/ui.rs` - Ratatui TUI rendering (header, agent list, detail view, status bar)
- `src/agent.rs` - Agent struct, status, spawn logic with streaming
- `src/api.rs` - Anthropic API client (streaming + non-streaming)
- `src/orchestrator.rs` - Task decomposition via Claude (breaks task into parallel subtasks)

## Build & Run

```bash
export ANTHROPIC_API_KEY=sk-ant-...
cargo run
```

## Key Bindings

- **Input mode**: Type task, Enter to submit
- **Running mode**: Up/Down to select agent, j/k to scroll output
- **Done mode**: s to synthesize results, n for new task, q to quit
- Ctrl+C always quits

## How it works

1. User enters a high-level task
2. Orchestrator (Claude) decomposes it into 2-5 independent subtasks
3. All subtasks spawn as concurrent agents (tokio tasks) hitting the Anthropic streaming API simultaneously
4. TUI shows real-time streaming output from all agents in parallel
5. When all agents finish, user can synthesize results into a unified response
