import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { execFileSync } from 'node:child_process'
import { Context } from '@deepseek-ai/cordis'
import AgentRegistry from '@deepseek-ai/dsh-agent'
import AgentLoop from '@deepseek-ai/dsh-agent-loop'
import LlmRuntime, { LlmAdapter, createUserMessage, createToolResultMessage, ToolCallId } from '@deepseek-ai/dsh-llm'
import SessionProjectionRegistry from '@deepseek-ai/dsh-session-projection'
import SessionStore, { SessionId } from '@deepseek-ai/dsh-session'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime, { defineContentToolFixture } from '@deepseek-ai/dsh-tools'
const plugin = await import(process.env.REMEM_DSH_PLUGIN_MODULE ?? '../dist/index.js')

class ScriptedModel extends LlmAdapter {
  requests = []
  async *stream(options) {
    this.requests.push(options)
    const id = ToolCallId('fixture-call')
    const block = this.requests.length === 1
      ? { type: 'tool-call', id, name: 'fixture_probe', arguments: '{}' }
      : { type: 'text', text: 'Verified the DSH fixture result.' }
    yield { type: 'block-start', index: 0, blockType: block.type }
    yield { type: 'block-end', index: 0, block }
    yield { type: 'finish', reason: { kind: block.type === 'tool-call' ? 'tool-calls' : 'stop' } }
  }
}

export async function harness(executable) {
  const ctx = new Context()
  await ctx.plugin(LlmRuntime)
  await ctx.plugin(SessionStore)
  await ctx.plugin(SessionProjectionRegistry)
  await ctx.plugin(SystemPrompt)
  await ctx.plugin(ToolRuntime)
  await ctx.plugin(AgentRegistry)
  await ctx.plugin(AgentLoop, { agents: [] })
  const model = new ScriptedModel()
  ctx.llm.registerAdapter(['fixture'], model)
  ctx.tools.register(defineContentToolFixture({ name: 'fixture_probe', description: 'Fixture probe',
    parameters: {}, async execute() { return [{ type: 'text', text: 'fixture probe succeeded' }] } }))
  const fiber = await ctx.plugin(plugin, { executable })
  return { ctx, model, fiber }
}

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'dsh-remem-test-'))
  const executable = join(root, 'remem')
  const log = join(root, 'calls.jsonl')
  await writeFile(executable, `#!${process.execPath}\nimport fs from 'node:fs';\nconst args = process.argv.slice(2);\nlet data=''; for await (const c of process.stdin) data += c;\nfs.appendFileSync(${JSON.stringify(log)}, JSON.stringify({args,payload:data ? JSON.parse(data):null})+'\\n');\nif (args[0] === 'context') process.stdout.write('fixture project memory');\nif (args[0] === 'observe') await new Promise(r=>setTimeout(r, 25));\n`, { mode: 0o700 })
  return { root, executable, log, async calls() { return (await readFile(log, 'utf8')).trim().split('\n').map(JSON.parse) } }
}

export async function runTurn(ctx, id = 'dsh-fixture') {
  const handle = await ctx.agents.create({ sessionId: SessionId(id), meta: { cwd: tmpdir() },
    agentOptions: { provider: 'fixture', model: 'fixture' } })
  handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'Check the fixture probe.' }], source: { kind: 'user' } }))
  await handle.agent.whenIdle()
  return handle
}

test('real AgentLoop captures human/tool/assistant, injects once and flushes in order', async () => {
  const f = await fixture()
  const { ctx, model, fiber } = await harness(f.executable)
  try {
    const handle = await runTurn(ctx)
    await ctx.parallel('session/flush', handle.agent.session)
    const calls = await f.calls()
    assert.equal(calls.filter(c => c.args[0] === 'context').length, 1)
    const capture = calls.filter(c => c.payload)
    assert.deepEqual(capture.map(c => c.args[0]), ['session-init', 'observe', 'observe', 'summarize'])
    assert.equal(capture[0].payload.prompt, 'Check the fixture probe.')
    assert.equal(capture[1].payload.tool_name, 'fixture_probe')
    assert.equal(capture[2].payload.tool_name, 'assistant/message')
    assert.equal(capture[3].payload.last_assistant_message, 'Verified the DSH fixture result.')
    assert(capture.every(c => c.payload.host === 'deepseek-harness'))
    assert(model.requests[0].messages.some(m => m.source.kind === 'remem'))
    await handle.dispose()
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})

test('plugin unload drains pending real subprocess work without an explicit flush', async () => {
  const f = await fixture()
  const { ctx, fiber } = await harness(f.executable)
  try {
    const handle = await runTurn(ctx, 'unload-fixture')
    await fiber.dispose()
    assert.equal((await f.calls()).at(-1).args[0], 'summarize')
    await handle.dispose()
  } finally { await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})

test('capture failure reaches flush and request preparation instead of vanishing', async () => {
  const f = await fixture()
  await writeFile(f.executable, `#!${process.execPath}\nif (process.argv[2] === '--version') process.exit(0);\nprocess.exit(9);\n`, { mode: 0o700 })
  const { ctx, fiber, model } = await harness(f.executable)
  try {
    const handle = await runTurn(ctx, 'failure-fixture')
    await assert.rejects(ctx.parallel('session/flush', handle.agent.session), error =>
      error instanceof AggregateError && error.errors.some(e => /failed \(exit 9/.test(e.message)))
    assert.equal(model.requests.length, 0)
    await handle.dispose().catch(() => {})
  } finally { await fiber.dispose().catch(() => {}); await ctx.fiber.dispose().catch(() => {}); await rm(f.root, { recursive: true, force: true }) }
})

test('missing executable rejects plugin activation', async () => {
  const ctx = new Context()
  await assert.rejects(plugin.apply(ctx, { executable: '/missing/dsh-remem-binary' }), /Cannot start remem/)
  await ctx.fiber.dispose()
})


test('unload awaits all sessions even when one capture failed', async () => {
  const f = await fixture()
  await writeFile(f.executable, `#!${process.execPath}\nimport fs from 'node:fs';\nlet data=''; for await(const c of process.stdin) data += c;\nconst payload=data ? JSON.parse(data) : null;\nconst command=process.argv[2];\nif(payload?.session_id==='bad-drain') process.exit(9);\nif(command==='context') process.stdout.write('fixture memory');\nif(command==='summarize') await new Promise(r=>setTimeout(r,400));\nfs.appendFileSync(${JSON.stringify(f.log)},JSON.stringify({args:[command],payload})+'\\n');\n`, { mode: 0o700 })
  const { ctx, fiber } = await harness(f.executable)
  try {
    const bad = await runTurn(ctx, 'bad-drain')
    const good = await runTurn(ctx, 'good-drain')
    await fiber.dispose().catch(() => {})
    assert((await f.calls()).some(c => c.args[0] === 'summarize' && c.payload?.session_id === 'good-drain'))
    await bad.dispose().catch(() => {})
    await good.dispose()
  } finally { await ctx.fiber.dispose().catch(() => {}); await rm(f.root, { recursive: true, force: true }) }
})


test('real remem binary captures distinct DSH host and queues turn distillation', {
  skip: !process.env.REMEM_DSH_BINARY,
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'dsh-remem-live-'))
  const previousData = process.env.REMEM_DATA_DIR
  const previousConfig = process.env.REMEM_CONFIG
  const previousPlaintext = process.env.REMEM_ALLOW_PLAINTEXT_DB
  process.env.REMEM_DATA_DIR = root
  process.env.REMEM_CONFIG = join(root, 'config.toml')
  process.env.REMEM_ALLOW_PLAINTEXT_DB = '1'
  const executable = process.env.REMEM_DSH_BINARY
  // No provider credentials: the smoke verifies capture/queueing, not AI promotion.
  execFileSync(executable, ['cleanup', '--dry-run'], { stdio: 'ignore' })
  const { ctx, fiber, model } = await harness(executable)
  try {
    const handle = await runTurn(ctx, 'live-remem-smoke')
    await ctx.parallel('session/flush', handle.agent.session)
    assert.equal(model.requests.length, 2)
    assert(model.requests[0].messages.some(message => message.source.kind === 'remem'))
    const report = JSON.parse(execFileSync('/usr/bin/python3', ['-c', `import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1])
e=c.execute("SELECT h.name,e.event_type,e.tool_name,e.content_text FROM captured_events e JOIN hosts h ON h.id=e.host_id WHERE e.session_id='live-remem-smoke' ORDER BY e.id").fetchall()
t=c.execute("SELECT h.name,t.task_kind FROM extraction_tasks t JOIN hosts h ON h.id=t.host_id JOIN sessions s ON s.id=t.session_row_id WHERE s.session_id='live-remem-smoke'").fetchall()
print(json.dumps({'events':e,'tasks':t}))`, join(root, 'remem.db')], { encoding: 'utf8' }))
    assert(report.events.every(row => row[0] === 'deepseek-harness'))
    assert(report.events.some(row => row[1] === 'user_prompt_submit' && row[3].includes('Check the fixture probe.')))
    assert(report.events.some(row => row[2] === 'fixture_probe' && row[3].includes('fixture probe succeeded')))
    assert(report.events.some(row => row[2] === 'assistant/message' && row[3].includes('Verified the DSH fixture result.')))
    assert(report.events.some(row => row[1] === 'session_stop'))
    assert(report.tasks.some(row => row[0] === 'deepseek-harness' && row[1] === 'session_rollup'))
    console.log(`Real remem smoke: ${report.events.length} DSH capture events; ${report.tasks.length} extraction tasks; flush complete`)
    await handle.dispose()
  } finally {
    await fiber.dispose(); await ctx.fiber.dispose()
    for (const [key, value] of [['REMEM_DATA_DIR', previousData], ['REMEM_CONFIG', previousConfig], ['REMEM_ALLOW_PLAINTEXT_DB', previousPlaintext]]) {
      if (value === undefined) delete process.env[key]; else process.env[key] = value
    }
    // Background remem workers own their queues; leave this isolated scratch store for diagnostics.
    console.log(`Isolated smoke store: ${root}`)
  }
})


test('a post-commit orphan tool result remains a visible flush error', async () => {
  const f = await fixture()
  const { ctx, fiber } = await harness(f.executable)
  try {
    const session = ctx.sessions.create(SessionId('orphan-result'), { meta: { cwd: tmpdir() } })
    session.append('tool/result', { turn: 1, step: 1, message: createToolResultMessage({
      callId: ToolCallId('orphan-call'), content: [{ type: 'text', text: 'orphan evidence' }], isError: false,
    }) }, { surfaceOp: 'append' })
    await assert.rejects(ctx.parallel('session/flush', session), error =>
      error instanceof AggregateError && error.errors.some(e => /has no live call/.test(e.message)))
  } finally { await fiber.dispose().catch(() => {}); await ctx.fiber.dispose().catch(() => {}); await rm(f.root, { recursive: true, force: true }) }
})
