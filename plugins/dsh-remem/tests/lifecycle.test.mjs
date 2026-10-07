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
    assert.deepEqual(capture[3].payload.reason, { kind: 'completed' })
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
    let cancelFirst = true
    ctx.on('agent/pre-step', async ({ agent }, next) => {
      const decision = await next()
      if (cancelFirst && decision.kind === 'enter' && decision.messages.some(m => m.source.kind === 'remem')) {
        cancelFirst = false
        agent.cancel({ kind: 'user' })
      }
      return decision
    }, { prepend: true })
    const handle = await runTurn(ctx, 'live-remem-smoke')
    assert.equal(model.requests.length, 0)
    assert.equal(handle.agent.session.deriveMessages().filter(m => m.source.kind === 'remem').length, 0)
    handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'Retry the fixture probe.' }], source: { kind: 'user' } }))
    await handle.agent.whenIdle()
    await ctx.parallel('session/flush', handle.agent.session)
    assert.equal(model.requests.length, 2)
    assert.equal(handle.agent.session.deriveMessages().filter(m => m.source.kind === 'remem').length, 1)
    handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'Continue with unchanged memory.' }], source: { kind: 'user' } }))
    await handle.agent.whenIdle()
    await ctx.parallel('session/flush', handle.agent.session)
    assert.equal(handle.agent.session.deriveMessages().filter(m => m.source.kind === 'remem').length, 1)
    const attachmentSession = ctx.sessions.create(SessionId('live-attachments'), { meta: { cwd: tmpdir() } })
    const image = { type: 'image', attachment: { attachmentId: 'live-image', name: 'probe.png', mediaType: 'image/png', bytes: 10, width: 1, height: 1 } }
    const file = { type: 'file', attachment: { attachmentId: 'live-file', name: 'probe.txt', bytes: 12 } }
    for (const content of [[image, file], [{ type: 'text', text: 'mixed attachment prompt' }, image]]) {
      attachmentSession.append('user/message', createUserMessage({ content, source: { kind: 'user' } }), { surfaceOp: 'append' })
    }
    await ctx.parallel('session/flush', attachmentSession)
    assert(model.requests[0].messages.some(message => message.source.kind === 'remem'))
    const report = JSON.parse(execFileSync('/usr/bin/python3', ['-c', `import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1])
e=c.execute("SELECT h.name,e.event_type,e.tool_name,e.content_text FROM captured_events e JOIN hosts h ON h.id=e.host_id WHERE e.session_id='live-remem-smoke' ORDER BY e.id").fetchall()
t=c.execute("SELECT h.name,t.task_kind FROM extraction_tasks t JOIN hosts h ON h.id=t.host_id JOIN sessions s ON s.id=t.session_row_id WHERE s.session_id='live-remem-smoke'").fetchall()
a=c.execute("SELECT content_text FROM captured_events WHERE session_id='live-attachments' ORDER BY id").fetchall()
i=c.execute("SELECT COUNT(*) FROM context_injection_items WHERE session_id IN ('live-remem-smoke','live-attachments') AND channel='prompt_submit'").fetchone()[0]
g=c.execute("SELECT COUNT(*) FROM context_injections WHERE session_id='live-remem-smoke'").fetchone()[0]
print(json.dumps({'events':e,'tasks':t,'attachments':a,'prompt_injections':i,'persisted_gates':g}))`, join(root, 'remem.db')], { encoding: 'utf8' }))
    assert.equal(report.attachments.length, 2)
    assert(report.attachments[0][0].includes('[image attachment'))
    assert(report.attachments[0][0].includes('[file attachment'))
    assert(report.attachments[1][0].includes('mixed attachment prompt'))
    assert.equal(report.prompt_injections, 0)
    assert.equal(report.persisted_gates, 0)
    assert(report.events.every(row => row[0] === 'deepseek-harness'))
    assert(report.events.some(row => row[1] === 'user_prompt_submit' && row[3].includes('Retry the fixture probe.')))
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

test('context injection preserves a downstream request-series declaration', async () => {
  const f = await fixture()
  const { ctx, fiber } = await harness(f.executable)
  let observed
  ctx.on('agent/pre-step', async (_, next) => {
    const decision = await next()
    return decision.kind === 'enter' ? { ...decision, startsRequestSeries: true } : decision
  })
  ctx.on('agent/pre-step', async (_, next) => {
    const decision = await next()
    if (decision.kind === 'enter' && decision.messages.some(m => m.source.kind === 'remem')) observed = decision
    return decision
  }, { prepend: true })
  try {
    const handle = await runTurn(ctx, 'request-series-fixture')
    await ctx.parallel('session/flush', handle.agent.session)
    assert.equal(observed?.startsRequestSeries, true)
    await handle.dispose()
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})

test('turn-end reasons survive capture when no assistant answer was produced', async () => {
  const f = await fixture()
  const { ctx, fiber } = await harness(f.executable)
  const reasons = [{ kind: 'blocked' }, { kind: 'aborted', reason: { kind: 'user' } },
    { kind: 'error', error: { message: 'fixture failure', code: 'UNKNOWN' } }]
  try {
    const session = ctx.sessions.create(SessionId('turn-reason-fixture'), { meta: { cwd: tmpdir() } })
    for (const [index, reason] of reasons.entries()) {
      session.append('turn/start', { turn: index + 1 })
      session.append('turn/end', { turn: index + 1, reason })
    }
    await ctx.parallel('session/flush', session)
    const summaries = (await f.calls()).filter(c => c.args[0] === 'summarize')
    assert.deepEqual(summaries.map(c => c.payload.reason), reasons)
    assert(summaries.every(c => c.payload.last_assistant_message === undefined))
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})

test('attachment-only and mixed human prompts retain bounded references without bytes', async () => {
  const f = await fixture()
  const { ctx, fiber } = await harness(f.executable)
  try {
    const session = ctx.sessions.create(SessionId('attachment-fixture'), { meta: { cwd: tmpdir() } })
    const image = { type: 'image', attachment: { attachmentId: 'image-ref', name: 'probe.png', mediaType: 'image/png', bytes: 10, width: 1, height: 1,
      path: '/synthetic/probe.png', data: 'INLINE_BYTES_MUST_NOT_CAPTURE' } }
    const file = { type: 'file', attachment: { attachmentId: 'file-ref', name: 'x'.repeat(1024), bytes: 12,
      path: 'data:INLINE_URI_MUST_NOT_CAPTURE' } }
    for (const content of [[image, file], [{ type: 'text', text: 'mixed prompt' }, image]]) {
      session.append('user/message', createUserMessage({ content, source: { kind: 'user' } }), { surfaceOp: 'append' })
    }
    await ctx.parallel('session/flush', session)
    const prompts = (await f.calls()).filter(c => c.args[0] === 'session-init').map(c => c.payload.prompt)
    assert(prompts[0].includes('[image attachment'))
    assert(prompts[0].includes('[file attachment'))
    assert(prompts[0].includes('image-ref'))
    assert(prompts[0].length < 1024)
    assert(!prompts[0].includes('INLINE_'))
    assert(prompts[1].startsWith('mixed prompt\n[image attachment'))
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})

test('cancelled preparation retries; only committed unchanged snapshots suppress injection', async () => {
  const f = await fixture()
  const { ctx, fiber, model } = await harness(f.executable)
  let cancelFirst = true
  // Outer middleware cancels after remem has returned its proposed messages,
  // reproducing the admission race beyond the plugin's own signal check.
  ctx.on('agent/pre-step', async ({ agent }, next) => {
    const decision = await next()
    if (cancelFirst && decision.kind === 'enter' && decision.messages.some(m => m.source.kind === 'remem')) {
      cancelFirst = false
      agent.cancel({ kind: 'user' })
    }
    return decision
  }, { prepend: true })
  try {
    const handle = await runTurn(ctx, 'cancel-retry-fixture')
    assert.equal(model.requests.length, 0)
    assert.equal(handle.agent.session.deriveMessages().filter(m => m.source.kind === 'remem').length, 0)
    handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'retry' }], source: { kind: 'user' } }))
    await handle.agent.whenIdle()
    assert.equal(model.requests.length, 2)
    assert.equal(handle.agent.session.deriveMessages().filter(m => m.source.kind === 'remem').length, 1)
    handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'unchanged later turn' }], source: { kind: 'user' } }))
    await handle.agent.whenIdle()
    assert.equal(handle.agent.session.deriveMessages().filter(m => m.source.kind === 'remem').length, 1)
    // New plugin instance must use committed history rather than private state.
    await fiber.dispose()
    const replacement = await ctx.plugin(plugin, { executable: f.executable })
    try {
      handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'after reload' }], source: { kind: 'user' } }))
      await handle.agent.whenIdle()
      assert.equal(handle.agent.session.deriveMessages().filter(m => m.source.kind === 'remem').length, 1)
      await ctx.parallel('session/flush', handle.agent.session)
      const contexts = (await f.calls()).filter(c => c.args[0] === 'context')
      assert.equal(contexts.length, 4)
      assert(contexts.every(c => c.args.at(-1) === 'off'))
      await handle.dispose()
    } finally { await replacement.dispose() }
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})
