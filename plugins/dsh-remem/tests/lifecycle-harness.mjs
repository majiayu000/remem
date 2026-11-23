import { Context } from '@deepseek-ai/cordis'
import AgentRegistry from '@deepseek-ai/dsh-agent'
import AgentLoop from '@deepseek-ai/dsh-agent-loop'
import LlmRuntime, { LlmAdapter, ToolCallId } from '@deepseek-ai/dsh-llm'
import SessionProjectionRegistry from '@deepseek-ai/dsh-session-projection'
import SessionStore from '@deepseek-ai/dsh-session'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime, { defineTool } from '@deepseek-ai/dsh-tools'
import * as NativeToolExecutor from '@deepseek-ai/dsh-tool-executor-native'

const plugin = await import(process.env.REMEM_DSH_PLUGIN_MODULE ?? '../dist/index.js')

class ScriptedModel extends LlmAdapter {
  requests = []
  toolName = 'fixture_probe'
  arguments = '{}'
  answer = 'Verified the DSH fixture result.'
  async *stream(options) {
    this.requests.push(options)
    const id = ToolCallId('fixture-call')
    const block = this.requests.length === 1
      ? { type: 'tool-call', id, name: this.toolName, arguments: this.arguments }
      : { type: 'text', text: this.answer }
    yield { type: 'block-start', index: 0, blockType: block.type }
    yield { type: 'block-end', index: 0, block }
    yield { type: 'finish', reason: { kind: block.type === 'tool-call' ? 'tool-calls' : 'stop' } }
  }
}

export async function harness(executable, toolName = 'fixture_probe') {
  const ctx = new Context()
  await ctx.plugin(LlmRuntime)
  await ctx.plugin(SessionStore)
  await ctx.plugin(SessionProjectionRegistry)
  await ctx.plugin(SystemPrompt)
  await ctx.plugin(ToolRuntime)
  await ctx.plugin(NativeToolExecutor)
  await ctx.plugin(AgentRegistry)
  await ctx.plugin(AgentLoop, { agents: [] })
  const model = new ScriptedModel()
  model.toolName = toolName
  ctx.llm.registerAdapter(['fixture'], model)
  const toolArguments = []
  ctx.tools.register(defineTool({ name: toolName, description: 'Fixture probe',
    parameters: {},
    output: { schema: { type: 'string' }, render: (_args, value) => [{ type: 'text', text: value }] },
    async execute(args) {
      toolArguments.push(args)
      return 'fixture probe succeeded'
    } }))
  const fiber = await ctx.plugin(plugin, { executable })
  return { ctx, model, fiber, toolArguments }
}

