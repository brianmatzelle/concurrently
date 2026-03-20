import { createAgent, spawnAgent, type Agent, type AgentEvent, type AgentStatus } from './agent.ts'

const AGENT_COLORS = [
  0x5dadec, // Sky blue
  0xff6b6b, // Coral
  0x69db7c, // Mint green
  0xffd43b, // Gold
  0xda77f2, // Lavender
  0x20c997, // Teal
  0xff922b, // Orange
  0xff8787, // Pink
]

function formatTime(): string {
  return new Date().toLocaleTimeString('en-US', { hour12: false })
}

function isError(status: AgentStatus): boolean {
  return typeof status === 'object' && 'error' in status
}

function isDone(status: AgentStatus): boolean {
  return status === 'done' || isError(status)
}

export interface LogEntry {
  text: string
  color: number
}

class AppState {
  input = $state('')
  agents = $state<Agent[]>([])
  scrollOffset = $state(0)
  log = $state<LogEntry[]>([])
  selectedAgentId = $state<string | null>(null)
  apiKey = ''

  private colorIndex = 0
  private agentLines = new Map<string, string>()

  init(apiKey: string) {
    this.apiKey = apiKey
    this.appendLog('concurrently — "name: task" to dispatch, /s to spawn more, Tab to switch agents', 0x555555)
  }

  cleanup() {}

  private nextColor(): number {
    const color = AGENT_COLORS[this.colorIndex % AGENT_COLORS.length]
    this.colorIndex++
    return color
  }

  get selectedAgent(): Agent | null {
    if (!this.selectedAgentId) return null
    return this.agents.find(a => a.id === this.selectedAgentId) ?? null
  }

  appendLog(text: string, color: number = 0xffffff) {
    this.log.push({ text, color })
    if (this.log.length > 500) {
      const removed = this.log.length - 400
      this.log = this.log.slice(-400)
      this.scrollOffset = Math.max(0, this.scrollOffset - removed)
    } else {
      this.log = [...this.log]
    }
  }

  private flushAgentLine(agent: Agent) {
    const remaining = this.agentLines.get(agent.id)
    if (remaining?.trim()) {
      this.appendLog(`[${agent.name}] ${remaining}`, agent.color)
    }
    this.agentLines.delete(agent.id)
  }

  submitInput() {
    const text = this.input.trim()
    if (!text) return
    this.input = ''

    // /s forces spawning a new agent
    if (text.startsWith('/s ')) {
      this.createNewAgent(text.slice(3).trim())
      return
    }

    // If an agent is selected (auto or manual), respond to it
    if (this.selectedAgentId) {
      const agent = this.agents.find(a => a.id === this.selectedAgentId)
      if (agent && isDone(agent.status)) {
        this.respondToAgent(this.selectedAgentId, text)
        return
      }
    }

    // No agents yet or none selected — spawn new
    this.createNewAgent(text)
  }

  createNewAgent(input: string) {
    let name: string
    let task: string
    const colonIdx = input.indexOf(':')
    if (colonIdx > 0 && colonIdx < 20) {
      name = input.slice(0, colonIdx).trim().toLowerCase().replace(/\s+/g, '-')
      task = input.slice(colonIdx + 1).trim()
      if (!task) {
        name = `agent-${this.agents.length + 1}`
        task = input
      }
    } else {
      name = `agent-${this.agents.length + 1}`
      task = input
    }

    const color = this.nextColor()
    const agent = createAgent(name, task, color)
    agent.conversation.push({ role: 'user', content: task })
    this.agents.push(agent)
    this.agents = [...this.agents]

    this.appendLog(`[${name}] dispatched: ${task}`, color)
    spawnAgent(agent, (event) => this.handleEvent(event))
  }

  respondToAgent(agentId: string, task: string) {
    const agent = this.agents.find(a => a.id === agentId)
    if (!agent || !isDone(agent.status)) return

    agent.conversation.push({ role: 'user', content: task })
    agent.task = task
    agent.status = 'queued'
    agent.output = ''
    agent.startedAt = null
    agent.finishedAt = null
    agent.tokensReceived = 0
    agent.currentTool = null
    agent.costUsd = 0

    this.appendLog(`[${agent.name}] → ${task}`, agent.color)
    this.selectedAgentId = null
    this.agents = [...this.agents]

    spawnAgent(agent, (event) => this.handleEvent(event))
  }

  selectNextAgent() {
    if (this.agents.length === 0) {
      this.selectedAgentId = null
      return
    }
    const currentIdx = this.agents.findIndex(a => a.id === this.selectedAgentId)
    const nextIdx = (currentIdx + 1) % this.agents.length
    this.selectedAgentId = this.agents[nextIdx].id
  }

  deselectAgent() {
    this.selectedAgentId = null
  }

  private handleEvent(event: AgentEvent) {
    if (event.type === 'finished') return

    const agent = this.agents.find(a => a.id === event.agentId)
    if (!agent) return

    switch (event.type) {
      case 'statusChange': {
        if (event.status === 'running' && !agent.startedAt) {
          agent.startedAt = formatTime()
        }
        if (event.status === 'done') {
          agent.finishedAt = formatTime()
          this.flushAgentLine(agent)
          agent.conversation.push({ role: 'assistant', content: agent.output })
          agent.totalCostUsd += agent.costUsd
          const cost = agent.costUsd > 0 ? ` $${agent.costUsd.toFixed(3)}` : ''
          this.appendLog(`[${agent.name}] done${cost}`, 0x00ff00)
          // Auto-select the last finished agent, but not if user is mid-typing
          if (!this.input) {
            this.selectedAgentId = agent.id
          }
        }
        if (isError(event.status)) {
          this.flushAgentLine(agent)
          if (agent.output) {
            agent.conversation.push({ role: 'assistant', content: agent.output })
          }
          const errMsg = typeof event.status === 'object' && 'error' in event.status
            ? event.status.error : 'unknown'
          this.appendLog(`[${agent.name}] error: ${errMsg}`, 0xff0000)
        }
        agent.status = event.status
        break
      }
      case 'textDelta': {
        agent.tokensReceived++
        agent.currentTool = null
        agent.output += event.text

        let current = this.agentLines.get(agent.id) ?? ''
        current += event.text
        const lines = current.split('\n')
        for (let i = 0; i < lines.length - 1; i++) {
          if (lines[i].trim()) {
            this.appendLog(`[${agent.name}] ${lines[i]}`, agent.color)
          }
        }
        this.agentLines.set(agent.id, lines[lines.length - 1])
        break
      }
      case 'toolUse': {
        this.flushAgentLine(agent)
        const label = event.detail ? `${event.tool}: ${event.detail}` : event.tool
        agent.currentTool = label
        agent.output += `\n[${label}]\n`
        this.appendLog(`[${agent.name}] ${label}`, agent.color)
        break
      }
      case 'costUpdate': {
        agent.costUsd = event.costUsd
        break
      }
    }
    this.agents = [...this.agents]
  }

  scrollUp() {
    this.scrollOffset += 3
  }

  scrollDown() {
    this.scrollOffset = Math.max(0, this.scrollOffset - 3)
  }
}

export const app = new AppState()
