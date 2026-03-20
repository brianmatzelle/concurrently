#!/usr/bin/env bun
import { mount } from 'sveltui'
import { mount as mountComponent } from 'svelte'
import App from './App.svelte'

const apiKey = process.env.ANTHROPIC_API_KEY
if (!apiKey) {
  console.error('Error: ANTHROPIC_API_KEY environment variable not set')
  console.error('  export ANTHROPIC_API_KEY=sk-ant-...')
  process.exit(1)
}

mount(() => {
  mountComponent(App, {
    target: document.body,
    props: { apiKey }
  })
}, { fullscreen: true })
