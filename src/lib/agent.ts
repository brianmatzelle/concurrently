export interface Message {
  role: string
  content: string
}

export type AgentStatus = 'queued' | 'running' | 'done' | { error: string }

export type AgentEvent =
  | { type: 'statusChange'; agentId: string; status: AgentStatus }
  | { type: 'textDelta'; agentId: string; text: string }
  | { type: 'toolUse'; agentId: string; tool: string; detail: string }
  | { type: 'costUpdate'; agentId: string; costUsd: number }
  | { type: 'finished'; agentId: string }

export interface Agent {
  id: string
  name: string
  task: string
  status: AgentStatus
  output: string
  startedAt: string | null
  finishedAt: string | null
  tokensReceived: number
  currentTool: string | null
  costUsd: number
  color: number
  conversation: Message[]
  totalCostUsd: number
}

export function createAgent(name: string, task: string, color: number): Agent {
  return {
    id: crypto.randomUUID().slice(0, 8),
    name,
    task,
    status: 'queued',
    output: '',
    startedAt: null,
    finishedAt: null,
    tokensReceived: 0,
    currentTool: null,
    costUsd: 0,
    color,
    conversation: [],
    totalCostUsd: 0,
  }
}

interface ParseState {
  agentId: string
  lastTextLen: number
  lastToolCount: number
}

function shortenPath(path: string): string {
  const parts = path.split('/')
  return parts.slice(-2).join('/')
}

function summarizeToolInput(toolName: string, input: any): string {
  if (!input) return ''
  switch (toolName) {
    case 'Read':
    case 'Edit':
    case 'Write':
      return input.file_path ? shortenPath(input.file_path) : ''
    case 'Bash':
      return input.command ? input.command.slice(0, 50) : ''
    case 'Glob':
      return input.pattern || ''
    case 'Grep':
      return input.pattern || ''
    default:
      return toolName
  }
}

function parseStreamLine(line: string, state: ParseState): AgentEvent[] {
  let v: any
  try {
    v = JSON.parse(line)
  } catch {
    return []
  }

  const msgType = v.type || ''

  if (msgType === 'assistant') {
    const events: AgentEvent[] = []
    const content = v.message?.content
    if (!Array.isArray(content)) return events

    let fullText = ''
    let toolCount = 0

    for (const block of content) {
      const blockType = block.type || ''
      if (blockType === 'text') {
        fullText += block.text || ''
      } else if (blockType === 'tool_use') {
        toolCount++
        if (toolCount > state.lastToolCount) {
          const name = block.name || 'unknown'
          const detail = summarizeToolInput(name, block.input)
          events.push({ type: 'toolUse', agentId: state.agentId, tool: name, detail })
        }
      }
    }

    state.lastToolCount = toolCount

    if (fullText.length > state.lastTextLen) {
      const newText = fullText.slice(state.lastTextLen)
      state.lastTextLen = fullText.length
      events.push({ type: 'textDelta', agentId: state.agentId, text: newText })
    }

    return events
  }

  if (msgType === 'result') {
    const events: AgentEvent[] = []
    const subtype = v.subtype || ''
    const cost = typeof v.cost_usd === 'number' ? v.cost_usd : null

    // Capture final text
    if (typeof v.result === 'string' && v.result.length > state.lastTextLen) {
      const newText = v.result.slice(state.lastTextLen)
      if (newText) {
        events.push({ type: 'textDelta', agentId: state.agentId, text: newText })
      }
    }
    state.lastTextLen = 0
    state.lastToolCount = 0

    if (cost !== null) {
      events.push({ type: 'costUpdate', agentId: state.agentId, costUsd: cost })
    }

    if (subtype === 'error') {
      const errorMsg = v.error || 'Unknown error'
      events.push({ type: 'statusChange', agentId: state.agentId, status: { error: errorMsg } })
    } else {
      events.push({ type: 'statusChange', agentId: state.agentId, status: 'done' })
    }

    events.push({ type: 'finished', agentId: state.agentId })
    return events
  }

  return []
}

/** Spawn a claude -p process and stream events via callback */
export function spawnAgent(
  agent: Agent,
  onEvent: (event: AgentEvent) => void,
): void {
  const { id: agentId, name, conversation } = agent

  // Current task is the last user message
  const currentTask = conversation[conversation.length - 1]?.content ?? agent.task

  // Build context from previous conversation rounds
  let systemCtx = `You are '${name}'. Complete your task concisely and directly. Current time: ${new Date().toISOString()}`

  const previousConversation = conversation.slice(0, -1)
  if (previousConversation.length > 0) {
    systemCtx += '\n\nPrevious conversation with your boss:\n'
    for (const msg of previousConversation) {
      const role = msg.role === 'user' ? 'boss' : 'you'
      systemCtx += `<${role}>\n${msg.content}\n</${role}>\n`
    }
  }

  onEvent({ type: 'statusChange', agentId, status: 'running' })

  const proc = Bun.spawn(
    [
      'claude', '-p', currentTask,
      '--append-system-prompt', systemCtx,
      '--output-format', 'stream-json',
      '--verbose',
      '--dangerously-skip-permissions',
      '--no-session-persistence',
      '--model', 'sonnet',
    ],
    {
      stdout: 'pipe',
      stderr: 'pipe',
      onExit: async (_proc, exitCode) => {
        if (exitCode !== 0 && exitCode !== null) {
          let stderrText = ''
          try {
            stderrText = await new Response(proc.stderr).text()
          } catch {}
          const detail = stderrText.trim() ? `: ${stderrText.trim().slice(0, 200)}` : ''
          onEvent({ type: 'statusChange', agentId, status: { error: `claude exited ${exitCode}${detail}` } })
          onEvent({ type: 'finished', agentId })
        }
      },
    },
  )

  // Read stdout line-by-line
  ;(async () => {
    const reader = proc.stdout.getReader()
    const decoder = new TextDecoder()
    let buffer = ''
    const state: ParseState = { agentId, lastTextLen: 0, lastToolCount: 0 }

    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true })

        const lines = buffer.split('\n')
        buffer = lines.pop() ?? ''

        for (const line of lines) {
          if (line.trim()) {
            const events = parseStreamLine(line, state)
            for (const event of events) {
              onEvent(event)
            }
          }
        }
      }

      // Process remaining buffer
      if (buffer.trim()) {
        const events = parseStreamLine(buffer, state)
        for (const event of events) {
          onEvent(event)
        }
      }
    } catch (e: any) {
      onEvent({ type: 'statusChange', agentId, status: { error: `Stream error: ${e.message}` } })
      onEvent({ type: 'finished', agentId })
    }
  })()
}
