<script>
  import { Box, Text, keyboard } from 'sveltui'
  import { onMount, onDestroy } from 'svelte'
  import { app } from './lib/state.svelte.ts'

  let { apiKey } = $props()

  onMount(() => {
    app.init(apiKey)
  })

  onDestroy(() => {
    app.cleanup()
  })

  const unsubCtrlC = keyboard.onKey('Ctrl+C', () => process.exit(0))
  const unsubTab = keyboard.onKey('Tab', () => app.selectNextAgent())

  const unsub = keyboard.on((event) => {
    // Escape: deselect agent
    if (event.key === 'Escape') {
      app.deselectAgent()
      return true
    }
    // Enter: submit input
    if (event.key === 'Enter' && app.input.length > 0) {
      app.submitInput()
      return true
    }
    // Backspace
    if (event.key === 'Backspace') {
      app.input = app.input.slice(0, -1)
      return true
    }
    // Scroll
    if (event.key === 'PageUp') { app.scrollUp(); return true }
    if (event.key === 'PageDown') { app.scrollDown(); return true }
    // Regular character input
    if (event.key.length === 1 && !event.ctrlKey && !event.altKey) {
      app.input += event.key
      return true
    }
    return false
  })

  onDestroy(() => {
    unsubCtrlC()
    unsubTab()
    unsub()
  })

  function agentIcon(status) {
    if (status === 'running') return '◉'
    if (status === 'done') return '●'
    if (status === 'queued') return '○'
    return '✗'
  }

  // Reserve 3 lines at bottom: agent bar + input + 1 safety
  const visibleCount = (process.stdout.rows ?? 24) - 3

  let visibleLog = $derived.by(() => {
    const all = app.log
    const offset = app.scrollOffset
    const end = all.length - offset
    const start = Math.max(0, end - visibleCount)
    return all.slice(Math.max(0, start), Math.max(0, end))
  })

  let agentStatusParts = $derived.by(() => {
    if (app.agents.length === 0) return null
    return app.agents.map(a => {
      const icon = agentIcon(a.status)
      const selected = a.id === app.selectedAgentId
      return {
        text: selected ? `[${icon} ${a.name}]` : ` ${icon} ${a.name} `,
        color: a.color,
      }
    })
  })

  let inputPrompt = $derived.by(() => {
    const sel = app.selectedAgent
    if (sel) {
      return { text: `> ${app.input}▌`, color: sel.color }
    }
    return { text: `> ${app.input}▌`, color: 0x00ffff }
  })
</script>

<Box width="100%" height="100%" flexDirection="column">
  <Box flexGrow={1} flexDirection="column">
    {#each visibleLog as entry}
      <Text text={entry.text} color={entry.color} />
    {/each}
  </Box>

  {#if agentStatusParts}
    <Box flexDirection="row">
      {#each agentStatusParts as part}
        <Text text={part.text} color={part.color} />
      {/each}
    </Box>
  {/if}

  <Text text={inputPrompt.text} color={inputPrompt.color} />
</Box>
