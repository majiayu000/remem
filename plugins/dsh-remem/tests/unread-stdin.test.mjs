import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import AgentRegistry from '@deepseek-ai/dsh-agent'
import AgentLoop from '@deepseek-ai/dsh-agent-loop'
import LlmRuntime, { createUserMessage } from '@deepseek-ai/dsh-llm'
import SessionProjectionRegistry from '@deepseek-ai/dsh-session-projection'
import SessionStore, { SessionId } from '@deepseek-ai/dsh-session'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
const plugin = await import(process.env.REMEM_DSH_PLUGIN_MODULE ?? '../dist/index.js')

// remem returns before reading stdin when REMEM_DISABLE_HOOKS=1 or when it fails
// early. A prompt larger than the pipe buffer then makes the adapter's write fail
// with EPIPE; the exit status, not the broken pipe, must decide the capture result.
const largePrompt = 'large prompt evidence\n'.repeat(24 * 1024)

async function capturePrompt(exitCode) {
  const root = await mkdtemp(join(tmpdir(), 'dsh-remem-unread-'))
  const executable = join(root, 'remem')
  await writeFile(executable, `#!${process.execPath}\nprocess.exit(process.argv[2] === '--version' ? 0 : ${exitCode});\n`, { mode: 0o700 })
  const ctx = new Context()
  for (const dependency of [LlmRuntime, SessionStore, SessionProjectionRegistry, SystemPrompt, ToolRuntime, AgentRegistry]) {
    await ctx.plugin(dependency)
  }
  await ctx.plugin(AgentLoop, { agents: [] })
  const fiber = await ctx.plugin(plugin, { executable })
  ctx.logger.error = () => {}
  try {
    const session = ctx.sessions.create(SessionId(`unread-stdin-${exitCode}`), { meta: { cwd: tmpdir() } })
    session.append('user/message', createUserMessage({ content: [{ type: 'text', text: largePrompt }],
      source: { kind: 'user' } }), { surfaceOp: 'append' })
    return await ctx.sessions.flush(session).then(() => null, error => error)
  } finally {
    await fiber.dispose().catch(() => {}); await ctx.fiber.dispose().catch(() => {})
    await rm(root, { recursive: true, force: true })
  }
}

const messages = error => (error instanceof AggregateError ? error.errors.flatMap(messages) : [error.message])

test('a capture that exits 0 without reading a large payload succeeds', async () => {
  assert(Buffer.byteLength(largePrompt) > 256 * 1024)
  const failure = await capturePrompt(0)
  assert.equal(failure, null, failure && messages(failure).join('; '))
})

test('a capture that fails without reading a large payload reports its exit status', async () => {
  const failure = await capturePrompt(9)
  assert(failure instanceof Error)
  assert.deepEqual(messages(failure), ['remem session-init failed (exit 9, signal none); inspect the remem log'])
})
