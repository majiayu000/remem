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
  arguments = '{}'
  async *stream(options) {
    this.requests.push(options)
    const id = ToolCallId('fixture-call')
    const block = this.requests.length === 1
      ? { type: 'tool-call', id, name: 'fixture_probe', arguments: this.arguments }
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
    assert.deepEqual(capture[1].payload.tool_input.arguments, {})
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

test('capture failure is reported once and later captures recover', async () => {
  const f = await fixture()
  const normal = await readFile(f.executable, 'utf8')
  await writeFile(f.executable, `#!${process.execPath}\nif (process.argv[2] === '--version') process.exit(0);\nprocess.exit(9);\n`, { mode: 0o700 })
  const { ctx, fiber } = await harness(f.executable)
  const errors = []
  ctx.logger.error = message => errors.push(message)
  try {
    const handle = await ctx.agents.create({ sessionId: SessionId('failure-fixture'), meta: { cwd: tmpdir() },
      agentOptions: { provider: 'fixture', model: 'fixture' } })
    const session = handle.agent.session
    const append = prompt => session.append('user/message', createUserMessage({ content: [{ type: 'text', text: prompt }], source: { kind: 'user' } }), { surfaceOp: 'append' })
    append('first capture')
    await assert.rejects(ctx.parallel('session/flush', session), error =>
      error instanceof AggregateError && error.errors.some(e => /failed \(exit 9/.test(e.message)))
    assert(errors.some(message => /failed \(exit 9/.test(message)))
    await writeFile(f.executable, normal, { mode: 0o700 })
    append('recovered capture')
    await ctx.parallel('session/flush', session)
    assert((await f.calls()).some(call => call.payload?.prompt === 'recovered capture'))
    handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'healthy followup' }], source: { kind: 'user' } }))
    await handle.agent.whenIdle()
    await ctx.parallel('session/flush', handle.agent.session)
    await handle.dispose()
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
  // Keep the real queue/profile/executor path, but never invoke a paid model.
  const executor = join(root, 'offline-codex')
  const executorCalls = join(root, 'executor-calls.jsonl')
  await writeFile(executor, `#!${process.execPath}\nimport fs from 'node:fs';\nlet prompt=''; for await (const chunk of process.stdin) prompt += chunk;\nconst observation = prompt.includes('Extract durable observations');\nconst rollup = prompt.includes('You summarize captured development-session evidence');\nfs.appendFileSync(${JSON.stringify(executorCalls)}, JSON.stringify({observation,rollup})+'\\n');\nif (!observation && !rollup) process.exit(19);\nconst response = observation ? JSON.stringify({no_observations:{reason:'offline profile resolution probe'}}) : '<summary>Offline profile resolution probe.</summary><structured_fields><request></request><decisions></decisions><learned></learned><next_steps></next_steps><preferences></preferences></structured_fields><segments></segments>';\nfs.writeFileSync(process.argv[process.argv.indexOf('--output-last-message')+1],response);\n`, { mode: 0o700 })
  await writeFile(process.env.REMEM_CONFIG, `[memory_ai.profiles.codex]\npath = ${JSON.stringify(executor)}\n`)
  execFileSync(executable, ['cleanup', '--dry-run'], { stdio: 'ignore' })
  const { ctx, fiber, model } = await harness(executable)
  model.arguments = JSON.stringify({ a: 1, password: 'hunter2' })
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
    let executorEvidence = []
    for (let attempt = 0; attempt < 200; attempt++) {
      executorEvidence = (await readFile(executorCalls, 'utf8').catch(() => '')).trim().split('\n').filter(Boolean).map(JSON.parse)
      if (executorEvidence.some(call => call.observation) && executorEvidence.some(call => call.rollup)) break
      // A hook fallback intentionally processes at most four work items.
      // Admit another real worker pass so follow-up jobs cannot starve the
      // queued observation in this isolated end-to-end fixture.
      execFileSync(executable, ['worker', '--once'], { stdio: 'ignore', timeout: 10_000 })
      await new Promise(resolve => setTimeout(resolve, 25))
    }
    assert(executorEvidence.some(call => call.observation), 'DSH observation queue did not reach the configured executor')
    assert(executorEvidence.some(call => call.rollup), 'DSH rollup queue did not reach the configured executor')
    assert(!(await readFile(process.env.REMEM_CONFIG, 'utf8')).includes('deepseek-harness'))
    const report = JSON.parse(execFileSync('/usr/bin/python3', ['-c', `import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1])
e=c.execute("SELECT h.name,e.event_type,e.tool_name,e.content_text FROM captured_events e JOIN hosts h ON h.id=e.host_id WHERE e.session_id='live-remem-smoke' ORDER BY e.id").fetchall()
t=c.execute("SELECT h.name,t.task_kind FROM extraction_tasks t JOIN hosts h ON h.id=t.host_id JOIN sessions s ON s.id=t.session_row_id WHERE s.session_id='live-remem-smoke'").fetchall()
a=c.execute("SELECT content_text FROM captured_events WHERE session_id='live-attachments' ORDER BY id").fetchall()
i=c.execute("SELECT COUNT(*) FROM context_injection_items WHERE session_id IN ('live-remem-smoke','live-attachments')").fetchone()[0]
g=c.execute("SELECT COUNT(*) FROM context_injections WHERE session_id='live-remem-smoke'").fetchone()[0]
print(json.dumps({'events':e,'tasks':t,'attachments':a,'prompt_injections':i,'persisted_gates':g}))`, join(root, 'remem.db')], { encoding: 'utf8' }))
    assert.equal(report.attachments.length, 2)
    assert(report.attachments[0][0].includes('[image attachment'))
    assert(report.attachments[0][0].includes('[file attachment'))
    assert(report.attachments[1][0].includes('mixed attachment prompt'))
    assert.equal(report.prompt_injections, 0)
    assert.equal(report.persisted_gates, 0)
    assert(report.events.every(row => row[0] === 'deepseek-harness'))
    assert(report.events.every(row => !row[3].includes('hunter2')))
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

test('an empty downstream first-step decision completes without memory-only dispatch', async () => {
  const f = await fixture()
  const { ctx, model, fiber } = await harness(f.executable)
  let emptyFirstStep = true
  ctx.on('agent/pre-step', async (_, next) => {
    const decision = await next()
    return emptyFirstStep && decision.kind === 'enter'
      ? { ...decision, messages: [], startsRequestSeries: true }
      : decision
  })
  try {
    const handle = await runTurn(ctx, 'empty-admission-fixture')
    const session = handle.agent.session
    await ctx.parallel('session/flush', session)
    assert.equal(model.requests.length, 0)
    assert.equal((await f.calls()).filter(call => call.args[0] === 'context').length, 0)
    assert.equal(session.snapshotEvents().filter(event => event.type === 'user/message'
      && event.data.source.kind === 'remem').length, 0)
    assert.deepEqual(session.snapshotEvents().filter(event => event.type === 'turn/end')
      .map(event => event.data.reason), [{ kind: 'completed' }])
    assert.equal(handle.agent.inbox.hasPending, false)

    emptyFirstStep = false
    handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'admit this followup' }], source: { kind: 'user' } }))
    await handle.agent.whenIdle()
    await ctx.parallel('session/flush', session)
    assert.equal(model.requests.length, 2)
    assert.equal((await f.calls()).filter(call => call.args[0] === 'context').length, 1)
    assert.equal(session.deriveMessages().filter(message => message.source.kind === 'remem').length, 1)
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


test('structured JSON arguments reach key redaction instead of an opaque string', async () => {
  const f = await fixture()
  const { ctx, fiber, model } = await harness(f.executable)
  model.arguments = JSON.stringify({ a: 1, password: 'fixture-secret' })
  try {
    const handle = await runTurn(ctx, 'structured-arguments')
    await ctx.parallel('session/flush', handle.agent.session)
    const call = (await f.calls()).find(c => c.payload?.tool_name === 'fixture_probe')
    assert.deepEqual(call.payload.tool_input.arguments, { a: 1, password: 'fixture-secret' })
    await handle.dispose()
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})

test('invalid JSON arguments fail visibly without copying argument text', async () => {
  const f = await fixture()
  const { ctx, fiber } = await harness(f.executable)
  try {
    const session = ctx.sessions.create(SessionId('invalid-arguments'), { meta: { cwd: tmpdir() } })
    const callId = ToolCallId('invalid-call')
    session.append('tool/call', { turn: 1, step: 1, callId, name: 'fixture_probe', arguments: '{"password":"fixture-secret"' })
    session.append('tool/result', { turn: 1, step: 1, message: createToolResultMessage({
      callId, content: [{ type: 'text', text: 'ordinary result' }], isError: false,
    }) }, { surfaceOp: 'append' })
    await assert.rejects(ctx.parallel('session/flush', session), error =>
      error instanceof AggregateError && error.errors.some(e => e.message === 'remem: DSH tool arguments are invalid JSON; capture rejected'))
    assert(!(await readFile(f.log, 'utf8')).includes('fixture-secret'))
    assert(!(await f.calls()).some(c => c.args[0] === 'observe'))
  } finally { await fiber.dispose().catch(() => {}); await ctx.fiber.dispose().catch(() => {}); await rm(f.root, { recursive: true, force: true }) }
})


test('blocked context cancellation promptly kills the real child without injection', async () => {
  const f = await fixture()
  const pidFile = join(f.root, 'context.pid')
  await writeFile(f.executable, `#!${process.execPath}\nimport fs from 'node:fs';\nif (process.argv[2] === 'context') { fs.writeFileSync(${JSON.stringify(pidFile)}, String(process.pid)); await new Promise(r=>setTimeout(r, 60000)); }\n`, { mode: 0o700 })
  const { ctx, fiber, model } = await harness(f.executable)
  try {
    const handle = await ctx.agents.create({ sessionId: SessionId('blocked-context'), meta: { cwd: tmpdir() },
      agentOptions: { provider: 'fixture', model: 'fixture' } })
    handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'cancel blocked context' }], source: { kind: 'user' } }))
    for (let attempt = 0; ; attempt++) {
      if (await readFile(pidFile, 'utf8').catch(() => '')) break
      assert(attempt < 200, 'context child did not start')
      await new Promise(resolve => setTimeout(resolve, 10))
    }
    const pid = Number(await readFile(pidFile, 'utf8'))
    const before = Date.now()
    handle.agent.cancel({ kind: 'user' })
    await handle.agent.whenIdle()
    assert(Date.now() - before < 2000, 'cancellation waited for the context timeout')
    assert.equal(model.requests.length, 0)
    assert.equal(handle.agent.session.deriveMessages().filter(m => m.source.kind === 'remem').length, 0)
    assert.throws(() => process.kill(pid, 0), { code: 'ESRCH' })
    assert.equal(handle.agent.inbox.hasPending, false)
    await handle.dispose()
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})

test('failed context restores the original claimed batch with provenance for one retry', async () => {
  const f = await fixture()
  const normal = await readFile(f.executable, 'utf8')
  await writeFile(f.executable, `#!${process.execPath}\nif (process.argv[2] === 'context') process.exit(9);\n`, { mode: 0o700 })
  const { ctx, fiber, model } = await harness(f.executable)
  const ends = []
  ctx.on('session/event', (_, event) => { if (event.type === 'turn/end') ends.push(event.data.reason) })
  try {
    const handle = await ctx.agents.create({ sessionId: SessionId('context-retry'), meta: { cwd: tmpdir() },
      agentOptions: { provider: 'fixture', model: 'fixture' } })
    const steering = createUserMessage({ content: [{ type: 'text', text: 'original steering' }], source: { kind: 'user' } })
    const original = createUserMessage({ content: [{ type: 'text', text: 'original prompt' }], source: { kind: 'user' } })
    handle.agent.inject(steering)
    handle.agent.followup(original)
    await handle.agent.whenIdle()
    assert.equal(model.requests.length, 0)
    assert(ends.some(reason => reason.kind === 'error' && /exit 9/.test(reason.error.message)))
    assert.deepEqual(handle.agent.inbox.nextStep, [steering, original])
    assert.equal(handle.agent.session.deriveMessages().filter(m => m.source.kind === 'user').length, 0)
    await writeFile(f.executable, normal, { mode: 0o700 })
    handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'retry now' }], source: { kind: 'user' } }))
    await handle.agent.whenIdle()
    await ctx.parallel('session/flush', handle.agent.session)
    const messages = handle.agent.session.deriveMessages()
    for (const expected of [steering, original]) {
      const committed = messages.filter(message => message.id === expected.id)
      assert.equal(committed.length, 1)
      assert.deepEqual(committed[0], expected)
      assert.equal(model.requests[0].messages.filter(message => message.id === expected.id).length, 1)
    }
    assert.equal(handle.agent.inbox.hasPending, false)
    assert.equal((await f.calls()).filter(call => call.payload?.prompt === 'original prompt').length, 1)
    await handle.dispose()
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})


test('changed and removed snapshots supersede only remem history across real turns', async () => {
  const f = await fixture()
  const memoryFile = join(f.root, 'memory.txt')
  await writeFile(memoryFile, 'old memory guidance')
  const original = await readFile(f.executable, 'utf8')
  await writeFile(f.executable, original.replace("process.stdout.write('fixture project memory')", `process.stdout.write(fs.readFileSync(${JSON.stringify(memoryFile)}, 'utf8'))`), { mode: 0o700 })
  const { ctx, fiber, model } = await harness(f.executable)
  let cancelChanged = false
  ctx.on('agent/pre-step', async ({ agent }, next) => {
    const decision = await next()
    if (cancelChanged && decision.kind === 'enter' && decision.messages.some(m => m.source.kind === 'remem')) {
      cancelChanged = false
      agent.cancel({ kind: 'user' })
    }
    return decision
  }, { prepend: true })
  try {
    const handle = await runTurn(ctx, 'changed-snapshots')
    const session = handle.agent.session
    const oldEvent = session.snapshotEvents().find(e => e.type === 'user/message' && e.data.source.kind === 'remem')
    const human = session.deriveMessages().find(m => m.source.kind === 'user')
    const followup = async prompt => {
      handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: prompt }], source: { kind: 'user' } }))
      await handle.agent.whenIdle()
    }
    const memories = request => request.messages.filter(m => m.source.kind === 'remem').map(m => m.content.map(b => b.text).join(''))
    assert.deepEqual(memories(model.requests.at(-1)), ['old memory guidance'])
    await writeFile(memoryFile, 'current memory guidance')
    cancelChanged = true
    const requestsBeforeCancel = model.requests.length
    await followup('cancelled change')
    assert.equal(model.requests.length, requestsBeforeCancel)
    await followup('retry changed memory')
    assert.deepEqual(memories(model.requests.at(-1)), ['current memory guidance'])
    assert(model.requests.at(-1).messages.every(m => m.content.length > 0), 'native empty replacements must be omitted from provider requests')
    assert.equal(model.requests.at(-1).messages.filter(m => m.id === human.id).length, 1)
    assert.deepEqual(session.snapshotEvents()[oldEvent.seq], oldEvent, 'raw snapshot provenance must remain intact')
    const committedCount = session.snapshotEvents().filter(e => e.type === 'user/message' && e.data.source.kind === 'remem').length
    await followup('unchanged memory')
    assert.deepEqual(memories(model.requests.at(-1)), ['current memory guidance'])
    assert.equal(session.snapshotEvents().filter(e => e.type === 'user/message' && e.data.source.kind === 'remem').length, committedCount)
    await writeFile(memoryFile, '')
    await followup('memory removed')
    assert.deepEqual(memories(model.requests.at(-1)), [])
    assert.deepEqual(session.snapshotEvents()[oldEvent.seq], oldEvent)
    assert(session.deriveMessages().some(m => m.id === human.id))
    await ctx.parallel('session/flush', session)
    await handle.dispose()
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})
