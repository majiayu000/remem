import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'
import { Context } from '@deepseek-ai/cordis'
import AgentRegistry from '@deepseek-ai/dsh-agent'
import InvariantRegistry from '@deepseek-ai/dsh-invariants'
import * as SessionInvariant from '@deepseek-ai/dsh-session/invariant'
import AgentLoop from '@deepseek-ai/dsh-agent-loop'
import LlmRuntime, { LlmAdapter, createUserMessage, createToolResultMessage, ToolCallId } from '@deepseek-ai/dsh-llm'
import SessionProjectionRegistry from '@deepseek-ai/dsh-session-projection'
import SessionStore, { SessionId } from '@deepseek-ai/dsh-session'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime, { defineContentToolFixture, defineTool } from '@deepseek-ai/dsh-tools'
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
  await ctx.plugin(InvariantRegistry)
  await ctx.plugin(SessionInvariant)
  await ctx.plugin(SessionProjectionRegistry)
  await ctx.plugin(SystemPrompt)
  await ctx.plugin(ToolRuntime)
  await ctx.plugin(AgentRegistry)
  await ctx.plugin(AgentLoop, { agents: [] })
  const model = new ScriptedModel()
  model.toolName = toolName
  ctx.llm.registerAdapter(['fixture'], model)
  const toolArguments = []
  ctx.tools.register(defineContentToolFixture({ name: toolName, description: 'Fixture probe',
    parameters: {}, async execute(args) {
      toolArguments.push(args)
      return [{ type: 'text', text: 'fixture probe succeeded' }]
    } }))
  const fiber = await ctx.plugin(plugin, { executable })
  return { ctx, model, fiber, toolArguments }
}

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'dsh-remem-test-'))
  const executable = join(root, 'remem')
  const log = join(root, 'calls.jsonl')
  await writeFile(executable, `#!${process.execPath}\nimport fs from 'node:fs';\nconst args = process.argv.slice(2);\nconst chunks=[]; for await (const c of process.stdin) chunks.push(c); const data=Buffer.concat(chunks).toString('utf8');\nfs.appendFileSync(${JSON.stringify(log)}, JSON.stringify({args,payload:data ? JSON.parse(data):null})+'\\n');\nif (args[0] === 'context') process.stdout.write('fixture project memory');\nif (args[0] === 'observe') await new Promise(r=>setTimeout(r, 25));\n`, { mode: 0o700 })
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

for (const oversized of [false, true]) test(`native source survives colliding tool names and projection: ${oversized}`, async () => {
  const f = await fixture()
  const { ctx, fiber, model } = await harness(f.executable, 'assistant/message')
  model.arguments = JSON.stringify({ dsh_event_type: 'assistant/message',
    notes: oversized ? 'tool argument evidence\n'.repeat(80000) : 'tool argument evidence' })
  if (oversized) model.answer = 'assistant evidence\n'.repeat(80000)
  let handle
  try {
    handle = await runTurn(ctx, `source-collision-${oversized}`)
    await ctx.parallel('session/flush', handle.agent.session)
    const observed = (await f.calls()).filter(c => c.args[0] === 'observe').map(c => c.payload)
    assert.equal(observed.length, 2)
    assert(observed.every(p => p.tool_name === 'assistant/message'))
    assert.deepEqual(observed.map(p => p.tool_input.dsh_event_type), ['tool/result', 'assistant/message'])
    assert.equal(observed[0].tool_input.arguments.dsh_event_type, 'assistant/message')
    assert(observed.every(p => Buffer.byteLength(JSON.stringify(p)) <= 1024 * 1024))
    if (oversized) assert(observed.every(p => p.tool_input.remem_capture.omitted))
  } finally {
    await handle?.dispose(); await fiber.dispose(); await ctx.fiber.dispose()
    await rm(f.root, { recursive: true, force: true })
  }
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

test('one checkpoint reports every queued async and synchronous capture failure', async () => {
  const f = await fixture()
  const normal = await readFile(f.executable, 'utf8')
  await writeFile(f.executable, `#!${process.execPath}\nfor await (const chunk of process.stdin) {}\nif (process.argv[2] === 'session-init') process.exit(7);\nif (process.argv[2] === 'summarize') process.exit(9);\n`, { mode: 0o700 })
  const { ctx, fiber } = await harness(f.executable)
  const logged = []
  ctx.logger.error = message => logged.push(message)
  try {
    const session = ctx.sessions.create(SessionId('multiple-failures'), { meta: { cwd: tmpdir() } })
    session.append('turn/start', { turn: 1 })
    session.append('step/start', { turn: 1, step: 1 })
    const appendPrompt = prompt => session.append('user/message', createUserMessage({
      content: [{ type: 'text', text: prompt }], source: { kind: 'user' },
    }), { surfaceOp: 'append' })
    appendPrompt('first capture')
    for (const [callId, name, args] of [['invalid', 'fixture_probe', 'private-invalid-json'],
      ['identity', 't'.repeat(1024 * 1024), '{}']]) {
      session.append('tool/call', { turn: 1, step: 1, callId: ToolCallId(callId), name, arguments: args })
      session.append('tool/result', { turn: 1, step: 1, message: createToolResultMessage({
        callId: ToolCallId(callId), content: [{ type: 'text', text: 'evidence' }], isError: false,
      }) }, { surfaceOp: 'append' })
    }
    session.append('step/end', { turn: 1, step: 1 })
    session.append('turn/end', { turn: 1, reason: { kind: 'completed' } })
    const expected = [
      'remem: DSH tool arguments are invalid JSON; capture rejected',
      'remem: tool observation identity exceeds the stdin budget',
      'remem session-init failed (exit 7, signal none); inspect the remem log',
      'remem summarize failed (exit 9, signal none); inspect the remem log',
    ].sort()
    const failure = await ctx.parallel('session/flush', session).then(() => null, error => error)
    assert.deepEqual(logged.toSorted(), expected, 'log each current cause, never repeat the first error')
    assert(failure instanceof AggregateError)
    const batch = failure.errors[0]
    assert(batch instanceof AggregateError)
    assert.deepEqual(batch.errors.map(error => error.message).sort(), expected)
    assert(expected.every(message => batch.message.includes(message)), 'host-visible message must retain every cause')
    await ctx.parallel('session/flush', session)
    await writeFile(f.executable, normal, { mode: 0o700 })
    appendPrompt('healthy capture after reported batch')
    await ctx.parallel('session/flush', session)
    assert((await f.calls()).some(call => call.payload?.prompt === 'healthy capture after reported batch'))
    assert.equal(logged.length, 4)
  } finally {
    await fiber.dispose().catch(() => {}); await ctx.fiber.dispose().catch(() => {})
    await rm(f.root, { recursive: true, force: true })
  }
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


test('real remem binary preserves DSH assistant and colliding tool provenance through rollup promotion', {
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
  const decision = 'Use source kind telemetry for summary promotion gate'
  const executor = join(root, 'offline-codex')
  const executorCalls = join(root, 'executor-calls.jsonl')
  await writeFile(executor, `#!${process.execPath}\nimport fs from 'node:fs';\nlet prompt=''; for await (const chunk of process.stdin) prompt += chunk;\nconst observation = prompt.includes('Extract durable observations');\nconst rollup = prompt.includes('You summarize captured development-session evidence');\nfs.appendFileSync(${JSON.stringify(executorCalls)}, JSON.stringify({observation,rollup})+'\\n');\nif (!observation && !rollup) process.exit(19);\nconst response = observation ? JSON.stringify({no_observations:{reason:'offline profile resolution probe'}}) : '<summary>Offline profile resolution probe.</summary><structured_fields><request></request><decisions>${decision}</decisions><learned></learned><next_steps></next_steps><preferences></preferences></structured_fields><segments></segments>';\nfs.writeFileSync(process.argv[process.argv.indexOf('--output-last-message')+1],response);\n`, { mode: 0o700 })
  await writeFile(process.env.REMEM_CONFIG, `[memory_ai.profiles.codex]\npath = ${JSON.stringify(executor)}\n`)
  execFileSync(executable, ['cleanup', '--dry-run'], { stdio: 'ignore' })
  const { ctx, fiber, model } = await harness(executable, 'assistant/message')
  model.arguments = JSON.stringify({ a: 1, password: 'hunter2', dsh_event_type: 'assistant/message' })
  model.answer = decision
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
    let report
    for (let attempt = 0; attempt < 200; attempt++) {
      report = JSON.parse(execFileSync('/usr/bin/python3', ['-c', `import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1])
e=c.execute("SELECT h.name,e.event_type,e.tool_name,e.content_text,e.role FROM captured_events e JOIN hosts h ON h.id=e.host_id WHERE e.session_id='live-remem-smoke' ORDER BY e.id").fetchall()
t=c.execute("SELECT h.name,t.task_kind,t.status,t.high_watermark_event_id,t.completed_event_id FROM extraction_tasks t JOIN hosts h ON h.id=t.host_id JOIN sessions s ON s.id=t.session_row_id WHERE s.session_id='live-remem-smoke'").fetchall()
a=c.execute("SELECT content_text FROM captured_events WHERE session_id='live-attachments' ORDER BY id").fetchall()
i=c.execute("SELECT COUNT(*) FROM context_injection_items WHERE session_id IN ('live-remem-smoke','live-attachments')").fetchone()[0]
g=c.execute("SELECT COUNT(*) FROM context_injections WHERE session_id='live-remem-smoke'").fetchone()[0]
p=c.execute("SELECT review_status,source_trust_class,auto_promote_block_reason FROM memory_candidates c WHERE instr(c.text,?)>0 AND EXISTS (SELECT 1 FROM json_each(c.evidence_event_ids) i JOIN captured_events e ON e.id=i.value WHERE e.session_id='live-remem-smoke' AND (e.role='assistant' OR e.tool_name='assistant/message'))", (sys.argv[2],)).fetchall()
m=c.execute("SELECT COUNT(*) FROM memories WHERE instr(content,?)>0", (sys.argv[2],)).fetchone()[0]
print(json.dumps({'candidates':p,'promoted_memories':m,'events':e,'tasks':t,'attachments':a,'prompt_injections':i,'persisted_gates':g}))`, join(root, 'remem.db'), decision], { encoding: 'utf8' }))
      if (report.candidates.length > 0) break
      // Executor-start logs can describe an earlier rollup without assistant
      // evidence. Keep admitting bounded worker passes until durable promotion
      // evidence exists; a once worker can exit while follow-up work is pending.
      execFileSync(executable, ['worker', '--once'], { stdio: 'ignore', timeout: 10_000 })
      await new Promise(resolve => setTimeout(resolve, 25))
    }
    assert.equal(report.attachments.length, 2)
    assert(report.attachments[0][0].includes('[image attachment'))
    assert(report.attachments[0][0].includes('[file attachment'))
    assert(report.attachments[1][0].includes('mixed attachment prompt'))
    assert.equal(report.prompt_injections, 0)
    assert.equal(report.persisted_gates, 0)
    assert(report.events.every(row => row[0] === 'deepseek-harness'))
    assert(report.events.every(row => !row[3].includes('hunter2')))
    assert(report.events.some(row => row[1] === 'user_prompt_submit' && row[3].includes('Retry the fixture probe.')))
    assert(report.events.some(row => row[1] === 'tool_result' && row[2] === 'assistant/message'
      && row[4] === null && row[3].includes('fixture probe succeeded')))
    assert(report.candidates.length > 0,
      `rollup must produce a useful review candidate; tasks=${JSON.stringify(report.tasks.slice(0, 12))}`)
    assert(report.candidates.every(row => row[0] === 'pending_review'
      && row[1] === 'external_content' && row[2] === 'source_trust_below_floor'),
    `assistant-only evidence must stay review-gated: ${JSON.stringify(report.candidates)}`)
    assert.equal(report.promoted_memories, 0)
    const assistants = report.events.filter(row => row[1] === 'message' && row[4] === 'assistant')
    assert.equal(assistants.length, 2, 'both repeated assistant messages must be captured')
    assert(assistants.every(row => row[2] === null && row[3].includes(decision)))
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
    session.append('turn/start', { turn: 1 })
    session.append('step/start', { turn: 1, step: 1 })
    session.append('tool/call', { turn: 1, step: 1, callId: ToolCallId('orphan-call'), name: 'fixture_probe', arguments: '{}' })
    // The call is valid DSH history but predates this plugin instance.
    await fiber.dispose()
    await ctx.plugin(plugin, { executable: f.executable })
    session.append('tool/result', { turn: 1, step: 1, message: createToolResultMessage({
      callId: ToolCallId('orphan-call'), content: [{ type: 'text', text: 'orphan evidence' }], isError: false,
    }) }, { surfaceOp: 'append' })
    await assert.rejects(ctx.parallel('session/flush', session), error =>
      error instanceof AggregateError && error.errors.some(e => /has no live call/.test(e.message)))
  } finally { await Promise.resolve(fiber.dispose()).catch(() => {}); await ctx.fiber.dispose().catch(() => {}); await rm(f.root, { recursive: true, force: true }) }
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

test('real AgentLoop captures empty tool arguments as an object and completes the turn', async () => {
  const f = await fixture()
  const { ctx, fiber, model, toolArguments } = await harness(f.executable)
  model.arguments = ''
  const errors = []
  ctx.logger.error = message => errors.push(message)
  try {
    const handle = await runTurn(ctx, 'empty-arguments')
    await ctx.parallel('session/flush', handle.agent.session)
    assert.deepEqual(toolArguments, [{}])
    assert.equal(model.requests.length, 2)
    const calls = await f.calls()
    const capturedTools = calls.filter(c => c.payload?.tool_name === 'fixture_probe')
    assert.equal(capturedTools.length, 1)
    assert.deepEqual(capturedTools[0].payload.tool_input.arguments, {})
    assert(calls.some(c => c.payload?.tool_name === 'assistant/message'))
    assert.deepEqual(calls.at(-1).payload.reason, { kind: 'completed' })
    assert.deepEqual(handle.agent.session.snapshotEvents().findLast(e => e.type === 'turn/end').data.reason,
      { kind: 'completed' })
    assert.deepEqual(errors, [])
    await handle.dispose()
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})

test('invalid JSON arguments fail visibly without copying argument text', async () => {
  const f = await fixture()
  const { ctx, fiber } = await harness(f.executable)
  try {
    const session = ctx.sessions.create(SessionId('invalid-arguments'), { meta: { cwd: tmpdir() } })
    session.append('turn/start', { turn: 1 })
    session.append('step/start', { turn: 1, step: 1 })
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


for (const viaNpm of [false, true]) test(`blocked context cancellation promptly kills the real child without injection${viaNpm ? ' through npm' : ''}`, async () => {
  const f = await fixture()
  const pidFile = join(f.root, 'context.pid')
  await writeFile(f.executable, `#!${process.execPath}\nimport fs from 'node:fs';\nif (process.argv[2] === 'context') { fs.writeFileSync(${JSON.stringify(pidFile)}, String(process.pid)); await new Promise(r=>setTimeout(r, 60000)); }\n`, { mode: 0o700 })
  const previousBinary = process.env.REMEM_NPM_BINARY
  if (viaNpm) process.env.REMEM_NPM_BINARY = f.executable
  const executable = viaNpm ? fileURLToPath(new URL('../../../npm/remem/bin/remem.js', import.meta.url)) : f.executable
  const { ctx, fiber, model } = await harness(executable)
  let pid
  try {
    const handle = await ctx.agents.create({ sessionId: SessionId('blocked-context'), meta: { cwd: tmpdir() },
      agentOptions: { provider: 'fixture', model: 'fixture' } })
    handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'cancel blocked context' }], source: { kind: 'user' } }))
    for (let attempt = 0; ; attempt++) {
      if (await readFile(pidFile, 'utf8').catch(() => '')) break
      assert(attempt < 200, 'context child did not start')
      await new Promise(resolve => setTimeout(resolve, 10))
    }
    pid = Number(await readFile(pidFile, 'utf8'))
    const before = Date.now()
    handle.agent.cancel({ kind: 'user' })
    await Promise.race([handle.agent.whenIdle(), new Promise((_, reject) => setTimeout(() => reject(new Error('cancellation did not stop the process tree')), 2000))])
    assert(Date.now() - before < 2000, 'cancellation waited for the context timeout')
    assert.equal(model.requests.length, 0)
    assert.equal(handle.agent.session.deriveMessages().filter(m => m.source.kind === 'remem').length, 0)
    assert.throws(() => process.kill(pid, 0), { code: 'ESRCH' })
    assert.equal(handle.agent.inbox.hasPending, false)
    await handle.dispose()
  } finally {
    if (pid) { try { process.kill(pid, 'SIGKILL') } catch (error) { if (error.code !== 'ESRCH') throw error } }
    await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true })
    if (previousBinary === undefined) delete process.env.REMEM_NPM_BINARY; else process.env.REMEM_NPM_BINARY = previousBinary
  }
})

for (const viaNpm of [false, true]) test(`plugin unload cancels an active context child and preserves its prompt for reinstall${viaNpm ? ' through npm' : ''}`, async () => {
  const f = await fixture()
  const normal = await readFile(f.executable, 'utf8')
  const pidFile = join(f.root, 'context.pid')
  await writeFile(f.executable, `#!${process.execPath}\nimport fs from 'node:fs';\nif (process.argv[2] === 'context') { fs.writeFileSync(${JSON.stringify(pidFile)}, String(process.pid)); await new Promise(r=>setTimeout(r, 60000)); }\n`, { mode: 0o700 })
  const previousBinary = process.env.REMEM_NPM_BINARY
  if (viaNpm) process.env.REMEM_NPM_BINARY = f.executable
  const executable = viaNpm ? fileURLToPath(new URL('../../../npm/remem/bin/remem.js', import.meta.url)) : f.executable
  const { ctx, fiber, model } = await harness(executable)
  let pid
  let handle, reinstalled
  try {
    handle = await ctx.agents.create({ sessionId: SessionId('unload-context'), meta: { cwd: tmpdir() },
      agentOptions: { provider: 'fixture', model: 'fixture' } })
    const original = createUserMessage({ content: [{ type: 'text', text: 'retain this prompt after unload' }], source: { kind: 'user' } })
    handle.agent.followup(original)
    for (let attempt = 0; ; attempt++) {
      if (await readFile(pidFile, 'utf8').catch(() => '')) break
      assert(attempt < 200, 'context child did not start')
      await new Promise(resolve => setTimeout(resolve, 10))
    }
    pid = Number(await readFile(pidFile, 'utf8'))
    const before = Date.now()
    await Promise.race([Promise.resolve(fiber.dispose()), new Promise((_, reject) => setTimeout(() => reject(new Error('unload did not stop the process tree')), 2000))])
    assert(Date.now() - before < 2000, 'unload waited for the context timeout')
    assert.throws(() => process.kill(pid, 0), { code: 'ESRCH' })
    await handle.agent.whenIdle()
    assert.equal(model.requests.length, 0)
    assert.equal(handle.agent.session.deriveMessages().filter(m => m.source.kind === 'remem').length, 0)
    assert.deepEqual(handle.agent.inbox.nextStep, [original])

    await writeFile(f.executable, normal, { mode: 0o700 })
    reinstalled = await ctx.plugin(plugin, { executable })
    handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'retry after reinstall' }], source: { kind: 'user' } }))
    await handle.agent.whenIdle()
    assert.equal(model.requests.length, 2)
    assert.equal(model.requests[0].messages.filter(message => message.id === original.id).length, 1)
    assert.equal(handle.agent.inbox.hasPending, false)
  } finally {
    if (pid) { try { process.kill(pid, 'SIGKILL') } catch (error) { if (error.code !== 'ESRCH') throw error } }
    if (previousBinary === undefined) delete process.env.REMEM_NPM_BINARY; else process.env.REMEM_NPM_BINARY = previousBinary
    handle?.agent.cancel({ kind: 'user' })
    await handle?.agent.whenIdle()
    await handle?.dispose()
    await reinstalled?.dispose()
    await fiber.dispose()
    await ctx.fiber.dispose()
    await rm(f.root, { recursive: true, force: true })
  }
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
    assert(session.surface.nodes.includes(oldEvent.seq), 'pre-step cancellation must not replace committed memory')
    assert.equal(handle.agent.inbox.hasPending, false)
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
    assert(session.snapshotEvents().filter(e => e.type === 'turn/end').every(e => e.data.reason.kind === 'completed' || e.data.reason.kind === 'aborted'))
    assert.deepEqual(session.snapshotEvents()[oldEvent.seq], oldEvent)
    assert(session.deriveMessages().some(m => m.id === human.id))
    await ctx.parallel('session/flush', session)
    await handle.dispose()
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})

for (const memory of ['updated memory', '']) {
  for (const fault of ['replacement', 'admission', 'request', 'prepared-call', 'cancel-before-request', 'cancel-after-request', 'model']) {
    test(`${memory ? 'changed' : 'removed'} snapshot ${fault} preserves official invariant and retry semantics`, async () => {
      const f = await fixture()
      const normal = await readFile(f.executable, 'utf8')
      const { ctx, fiber, model } = await harness(f.executable)
      let handle, stopFault
      try {
        handle = await runTurn(ctx, `snapshot-${fault}-${memory ? 'changed' : 'removed'}`)
        const session = handle.agent.session
        await ctx.parallel('session/flush', session)
        const old = session.snapshotEvents().find(e => e.type === 'user/message' && e.data.source.kind === 'remem')
        await writeFile(f.executable, normal.replace("process.stdout.write('fixture project memory')",
          `process.stdout.write(${JSON.stringify(memory)})`), { mode: 0o700 })
        const original = createUserMessage({ content: [{ type: 'text', text: `original ${fault}` }], source: { kind: 'user' } })
        const steering = createUserMessage({ content: [{ type: 'text', text: 'original steering' }], source: { kind: 'user' } })
        if (fault === 'replacement' || fault === 'admission') {
          stopFault = ctx.on('internal/dispatch', (_, name, args) => {
            if (name !== 'session/event' || args[0] !== session) return
            const event = args[1]
            if (fault === 'replacement' && event.type === 'developer/message' && event.data.message.source.kind === 'remem'
              || fault === 'admission' && event.type === 'user/message' && event.data.id === original.id) {
              throw new Error(`fixture ${fault} failure`)
            }
          })
        } else if (fault === 'prepared-call') {
          const prepareCall = model.prepareCall
          model.prepareCall = async () => { throw new Error('fixture prepared-call failure') }
          stopFault = () => { model.prepareCall = prepareCall }
        } else if (fault === 'model') {
          const stream = model.stream
          model.stream = async function* () { throw new Error('fixture model failure') }
          stopFault = () => { model.stream = stream }
        } else {
          stopFault = ctx.on('agent/request', async ({ agent }, next) => {
            if (fault === 'cancel-before-request') agent.cancel({ kind: 'user' })
            const config = await next()
            if (fault === 'request') throw new Error('fixture request failure')
            if (fault === 'cancel-after-request') agent.cancel({ kind: 'user' })
            return config
          }, { prepend: true })
        }
        handle.agent.inject(steering)
        handle.agent.followup(original)
        await handle.agent.whenIdle()
        await ctx.parallel('session/flush', session)
        const cancelled = fault.startsWith('cancel-')
        const end = session.snapshotEvents().findLast(e => e.type === 'turn/end')
        assert.equal(end.data.reason.kind, cancelled ? 'aborted' : 'error')
        if (!cancelled) assert.match(end.data.reason.error.message, /fixture .*failure/)
        const committed = session.snapshotEvents().filter(e => e.type === 'user/message' && e.data.id === original.id)
        assert.equal(committed.length, fault === 'model' ? 1 : 0)
        assert.deepEqual(handle.agent.inbox.nextStep, cancelled || fault === 'model' ? [] : fault === 'admission' ? [original] : [steering, original])
        assert.deepEqual(session.snapshotEvents()[old.seq], old, 'original snapshot provenance is immutable')
        if (fault === 'replacement' || fault === 'cancel-before-request') {
          assert(session.surface.nodes.includes(old.seq), 'failed or cancelled replacement must not consume the old snapshot')
        }
        stopFault()
        stopFault = undefined
        handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'explicit retry' }], source: { kind: 'user' } }))
        await handle.agent.whenIdle()
        await ctx.parallel('session/flush', session)
        assert.equal(session.snapshotEvents().findLast(e => e.type === 'turn/end').data.reason.kind, 'completed')
        assert.deepEqual(model.requests.at(-1).messages.filter(m => m.source.kind === 'remem')
          .map(m => m.content.map(b => b.text).join('')), memory ? [memory] : [])
        for (const input of [steering, original]) {
          const expected = cancelled ? 0 : 1
          assert.equal(model.requests.at(-1).messages.filter(m => m.id === input.id).length, expected)
          assert.equal(session.snapshotEvents().filter(e => e.type === 'user/message' && e.data.id === input.id).length, expected)
          assert.equal((await f.calls()).filter(c => c.payload?.prompt === input.content[0].text).length, expected)
        }
        assert.equal(handle.agent.inbox.hasPending, false)
      } finally {
        stopFault?.()
        await handle?.dispose()
        await fiber.dispose()
        await ctx.fiber.dispose()
        await rm(f.root, { recursive: true, force: true })
      }
    })
  }
}

for (const failCapture of [false, true]) test(`cancelling a capture checkpoint returns promptly and retains ${failCapture ? 'the flush error' : 'durability'}`, async () => {
  const f = await fixture()
  const entered = join(f.root, 'capture-entered')
  const release = join(f.root, 'capture-release')
  const normal = await readFile(f.executable, 'utf8')
  await writeFile(f.executable, normal.replace("if (args[0] === 'observe') await new Promise(r=>setTimeout(r, 25));",
    `if (args[0] === 'observe') { fs.writeFileSync(${JSON.stringify(entered)}, 'ready'); while (!fs.existsSync(${JSON.stringify(release)})) await new Promise(r=>setTimeout(r, 25)); ${failCapture ? 'process.exit(9)' : ''} }`) + (failCapture ? "\nif (args[0] === 'session-init') process.exit(7); if (args[0] === 'summarize') process.exit(8);\n" : ''), { mode: 0o700 })
  const { ctx, model, fiber } = await harness(f.executable)
  let handle
  try {
    handle = await ctx.agents.create({ sessionId: SessionId(`cancel-capture-${failCapture}`), meta: { cwd: tmpdir() },
      agentOptions: { provider: 'fixture', model: 'fixture' } })
    handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'cancel during capture checkpoint' }], source: { kind: 'user' } }))
    for (let attempt = 0; !(await readFile(entered, 'utf8').catch(() => '')); attempt++) {
      assert(attempt < 400, 'capture subprocess did not reach checkpoint')
      await new Promise(resolve => setTimeout(resolve, 10))
    }
    handle.agent.cancel({ kind: 'user' })
    const idleBeforeRelease = await Promise.race([handle.agent.whenIdle().then(() => true),
      new Promise(resolve => setTimeout(() => resolve(false), 2000))])
    await writeFile(release, 'release')
    await handle.agent.whenIdle()
    assert.equal(idleBeforeRelease, true, 'cancellation must not wait for durable capture to finish')
    assert.equal(model.requests.length, 1)
    assert.equal(handle.agent.inbox.hasPending, false)
    assert.equal(handle.agent.session.snapshotEvents().findLast(e => e.type === 'turn/end').data.reason.kind, 'aborted')
    if (failCapture) {
      await assert.rejects(ctx.parallel('session/flush', handle.agent.session), error => {
        const batch = error instanceof AggregateError && error.errors[0]
        return batch instanceof AggregateError && batch.errors.length === 3
          && [7, 8, 9].every(code => batch.errors.some(cause => cause.message.includes(`exit ${code}`)))
      })
    }
    await ctx.parallel('session/flush', handle.agent.session)
    assert.equal((await f.calls()).at(-1).args[0], 'summarize')
  } finally {
    await writeFile(release, 'release')
    await handle?.agent.whenIdle()
    await handle?.dispose().catch(() => {})
    await Promise.resolve(fiber.dispose()).catch(() => {})
    await Promise.resolve(ctx.fiber.dispose()).catch(() => {})
    await rm(f.root, { recursive: true, force: true })
  }
})


for (const fault of ['unload', 'downstream-error', 'partial-commit', 'cancel']) {
  test(`unload during pending request preserves original input identities: ${fault}`, async () => {
    const f = await fixture()
    const { ctx, fiber, model } = await harness(f.executable)
    const entered = Promise.withResolvers()
    const release = Promise.withResolvers()
    let handle, stop, reinstalled
    try {
      handle = await runTurn(ctx, `unload-request-${fault}`)
      const session = handle.agent.session
      await ctx.parallel('session/flush', session)
      const steering = createUserMessage({ content: [{ type: 'text', text: 'Keep the original steering.' }], source: { kind: 'user' } })
      const original = createUserMessage({ content: [{ type: 'text', text: 'Keep the original followup.' }], source: { kind: 'user' } })
      stop = ctx.on('agent/request', async (_, next) => {
        const config = await next()
        entered.resolve()
        await release.promise
        // This commit occurs after Cordis has removed remem's event listener.
        if (fault === 'partial-commit') session.append('user/message', steering, { surfaceOp: 'append' })
        if (fault === 'downstream-error') throw new Error('downstream preparation failed after unload')
        return config
      })
      handle.agent.inject(steering)
      handle.agent.followup(original)
      await entered.promise
      if (fault === 'cancel') handle.agent.cancel({ kind: 'user' })
      await fiber.dispose()
      release.resolve()
      await handle.agent.whenIdle()
      assert.equal(model.requests.length, 2, 'the interrupted request must not reach the model')
      assert.equal(session.snapshotEvents().findLast(e => e.type === 'turn/end').data.reason.kind,
        fault === 'cancel' ? 'aborted' : 'error')
      assert.deepEqual(handle.agent.inbox.nextStep,
        fault === 'cancel' ? [] : fault === 'partial-commit' ? [original] : [steering, original])
      stop()
      stop = undefined
      reinstalled = await ctx.plugin(plugin, { executable: f.executable })
      handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: 'Explicit retry after reload.' }], source: { kind: 'user' } }))
      await handle.agent.whenIdle()
      await ctx.parallel('session/flush', session)
      assert.equal(session.snapshotEvents().findLast(e => e.type === 'turn/end').data.reason.kind, 'completed')
      for (const input of [steering, original]) {
        const expected = fault === 'cancel' ? 0 : 1
        assert.equal(session.snapshotEvents().filter(e => e.type === 'user/message' && e.data.id === input.id).length, expected)
        assert.equal(model.requests.at(-1).messages.filter(m => m.id === input.id).length, expected)
      }
      assert.equal(handle.agent.inbox.hasPending, false)
    } finally {
      release.resolve()
      stop?.()
      await handle?.agent.whenIdle()
      await handle?.dispose()
      await reinstalled?.dispose()
      await fiber.dispose()
      await ctx.fiber.dispose()
      await rm(f.root, { recursive: true, force: true })
    }
  })
}

const largeToolCases = ['presentation', 'arguments', 'result', 'wide-escaped']
async function largeToolHarness(executable, kind) {
  const h = await harness(executable)
  const useful = 'Local check completed successfully.'
  const argumentsValue = kind === 'arguments'
    ? { password: 'argument-secret', operation: 'verify deployment', records: Array.from({ length: 25000 }, (_, i) => ({ path: `src/entry-${i}.ts`, status: 'verified', details: 'a'.repeat(64) })) }
    : {}
  const resultText = kind === 'result'
    ? `${useful}\n${'Checked source entry successfully.\n'.repeat(70000)}final verification complete\n`
    : useful
  const meta = kind === 'presentation' ? { details: 'x'.repeat(2 * 1024 * 1024), status: 'verified', password: 'metadata-secret' }
    : kind === 'wide-escaped' ? { records: Array.from({ length: 60000 }, () => ({ proof: 'passed\\quoted"\n证据', password: 'nested-secret' })) } : {}
  h.ctx.tools.register(defineTool({ name: 'bounded_probe', description: 'Bounded capture fixture', parameters: {},
    output: { schema: { type: 'string' }, render: () => [{ type: 'text', text: resultText }], presentationMeta: () => meta },
    async execute(args) { assert.deepEqual(args, argumentsValue); return 'ok' },
  }))
  h.model.stream = async function* (options) {
    this.requests.push(options)
    const block = this.requests.length === 1
      ? { type: 'tool-call', id: ToolCallId('bounded-call'), name: 'bounded_probe', arguments: JSON.stringify(argumentsValue) }
      : { type: 'text', text: 'Bounded tool evidence captured.' }
    yield { type: 'block-start', index: 0, blockType: block.type }
    yield { type: 'block-end', index: 0, block }
    yield { type: 'finish', reason: { kind: block.type === 'tool-call' ? 'tool-calls' : 'stop' } }
  }
  return h
}

for (const kind of largeToolCases) test(`tool envelope stays within serialized stdin budget: ${kind}`, async () => {
  const f = await fixture()
  const normal = await readFile(f.executable, 'utf8')
  await writeFile(f.executable, normal.replace('fs.appendFileSync(',
    "if (Buffer.byteLength(data) > 1024 * 1024) process.exit(7);\nfs.appendFileSync("), { mode: 0o700 })
  const { ctx, fiber, model } = await largeToolHarness(f.executable, kind)
  let handle
  try {
    handle = await runTurn(ctx, `bounded-${kind}`)
    await ctx.parallel('session/flush', handle.agent.session)
    assert.equal(model.requests.length, 2)
    assert.equal(handle.agent.session.snapshotEvents().findLast(e => e.type === 'turn/end').data.reason.kind, 'completed')
    const call = (await f.calls()).find(c => c.payload?.tool_name === 'bounded_probe')
    assert(call, 'useful tool evidence must reach observe')
    assert(Buffer.byteLength(JSON.stringify(call.payload)) <= 1024 * 1024)
    assert(JSON.stringify(call.payload.tool_response).includes('Local check completed successfully.'))
    assert.equal(call.payload.tool_input.remem_capture.omitted, true)
    assert(call.payload.tool_input.remem_capture.original_envelope_bytes > 1024 * 1024)
    if (kind === 'arguments') {
      assert.equal(call.payload.tool_input.arguments.operation, 'verify deployment')
      assert.equal(call.payload.tool_input.arguments.password, 'argument-secret', 'the core owns secret redaction')
      assert(call.payload.tool_input.arguments.records.length > 0)
    }
  } finally {
    await handle?.dispose()
    await fiber.dispose()
    await ctx.fiber.dispose()
    await rm(f.root, { recursive: true, force: true })
  }
})

test('real remem captures bounded large tools through core redaction and summary', {
  skip: !process.env.REMEM_DSH_BINARY,
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'dsh-remem-bounded-live-'))
  const previous = Object.fromEntries(['HOME', 'REMEM_DATA_DIR', 'REMEM_CONFIG', 'REMEM_ALLOW_PLAINTEXT_DB'].map(key => [key, process.env[key]]))
  Object.assign(process.env, { HOME: root, REMEM_DATA_DIR: root, REMEM_CONFIG: join(root, 'config.toml'), REMEM_ALLOW_PLAINTEXT_DB: '1' })
  const executor = join(root, 'offline-executor.mjs')
  await writeFile(executor, `#!${process.execPath}\nimport fs from 'node:fs';let prompt='';for await(const c of process.stdin)prompt+=c;fs.writeFileSync(process.argv[process.argv.indexOf('--output-last-message')+1],prompt.includes('Extract durable observations')?JSON.stringify({no_observations:{reason:'bounded capture fixture'}}):'<summary>Bounded tool verification.</summary><structured_fields><request></request><decisions></decisions><learned></learned><next_steps></next_steps><preferences></preferences></structured_fields><segments></segments>');\n`, { mode: 0o700 })
  await writeFile(process.env.REMEM_CONFIG, `[memory_ai.profiles.codex]\npath = ${JSON.stringify(executor)}\n`)
  try {
    execFileSync(process.env.REMEM_DSH_BINARY, ['cleanup', '--dry-run'], { stdio: 'ignore' })
    for (const kind of largeToolCases) {
      const { ctx, fiber, model } = await largeToolHarness(process.env.REMEM_DSH_BINARY, kind)
      let handle
      try {
        handle = await runTurn(ctx, `live-bounded-${kind}`)
        await ctx.parallel('session/flush', handle.agent.session)
        assert.equal(model.requests.length, 2)
        assert.equal(handle.agent.session.snapshotEvents().findLast(e => e.type === 'turn/end').data.reason.kind, 'completed')
        const rows = JSON.parse(execFileSync('/usr/bin/python3', ['-c', `import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1]); print(json.dumps(c.execute("SELECT e.event_type,e.tool_name,COALESCE(CAST(b.content_bytes AS TEXT),e.content_text),b.content_encoding FROM captured_events e LEFT JOIN event_blobs b ON b.id=e.content_blob_id WHERE e.session_id=? ORDER BY e.id",(sys.argv[2],)).fetchall()))`, join(root, 'remem.db'), `live-bounded-${kind}`], { encoding: 'utf8' }))
        const captured = rows.find(row => row[1] === 'bounded_probe')
        assert(captured, 'large valid tool result must be captured')
        // event blobs are part of the existing core capture path, not adapter storage.
        assert(captured[3] === null || captured[3] === 'plain')
        const content = captured[2]
        assert(content.includes('Local check completed successfully.'))
        assert(captured[2].includes('remem_capture'))
        assert(!JSON.stringify(rows).match(/argument-secret|metadata-secret|nested-secret/))
        assert(rows.some(row => row[0] === 'session_stop'))
      } finally {
        await handle?.dispose()
        await fiber.dispose()
        await ctx.fiber.dispose()
      }
    }
    const { ctx, fiber, model } = await harness(process.env.REMEM_DSH_BINARY)
    let handle
    try {
      const message = 'Useful large message evidence.\npassword=message-secret\n' + '证据😀"\\\tverified\n'.repeat(120000)
      model.answer = message
      handle = await ctx.agents.create({ sessionId: SessionId('live-large-messages'), meta: { cwd: tmpdir() },
        agentOptions: { provider: 'fixture', model: 'fixture' } })
      handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: message }], source: { kind: 'user' } }))
      await handle.agent.whenIdle()
      await ctx.parallel('session/flush', handle.agent.session)
      assert.equal(model.requests.length, 2)
      assert.equal(handle.agent.session.snapshotEvents().findLast(e => e.type === 'turn/end').data.reason.kind, 'completed')
      const rows = JSON.parse(execFileSync('/usr/bin/python3', ['-c', `import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1]); rows=c.execute("SELECT e.event_type,e.role,COALESCE(CAST(b.content_bytes AS TEXT),e.content_text) FROM captured_events e LEFT JOIN event_blobs b ON b.id=e.content_blob_id WHERE e.session_id='live-large-messages' ORDER BY e.id").fetchall()
print(json.dumps([[kind,role,'Useful large message evidence.' in content,'remem_capture' in content,'message-secret' in content] for kind,role,content in rows]))`, join(root, 'remem.db')], { encoding: 'utf8' }))
      for (const row of [rows.find(r => r[0] === 'user_prompt_submit'),
        rows.find(r => r[0] === 'message' && r[1] === 'assistant'), rows.find(r => r[0] === 'session_stop')]) {
        assert(row, 'prompt, assistant and Stop must all persist')
        assert(row[2], 'useful message evidence must survive the core writer')
        assert(row[3], 'omission metadata must survive the core writer')
      }
      assert(rows.every(row => !row[4]), 'core redaction must remove the fixture secret')
    } finally { await handle?.dispose(); await fiber.dispose(); await ctx.fiber.dispose() }
  } finally {
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[key]
      else process.env[key] = value
    }
    // Workers may still own the isolated queue; retain it for diagnostics.
    console.log(`Isolated bounded-tool store: ${root}`)
  }
})


for (const kind of ['multiline-escaped', 'single-line']) test(`large messages preserve bounded evidence through Stop: ${kind}`, async () => {
  const f = await fixture()
  const normal = await readFile(f.executable, 'utf8')
  await writeFile(f.executable, normal.replace('fs.appendFileSync(',
    "if (Buffer.byteLength(data) > 1024 * 1024) process.exit(7);\nfs.appendFileSync("), { mode: 0o700 })
  const { ctx, fiber, model } = await harness(f.executable)
  const body = kind === 'multiline-escaped'
    ? 'Useful message evidence.\n' + '证据😀\\quoted"\tcompleted\n'.repeat(100000)
    : 'private-token-'.repeat(180000)
  model.answer = body
  let handle
  try {
    handle = await ctx.agents.create({ sessionId: SessionId(`large-message-${kind}`), meta: { cwd: tmpdir() },
      agentOptions: { provider: 'fixture', model: 'fixture' } })
    handle.agent.followup(createUserMessage({ content: [{ type: 'text', text: body }], source: { kind: 'user' } }))
    await handle.agent.whenIdle()
    await ctx.parallel('session/flush', handle.agent.session)
    assert.equal(model.requests.length, 2)
    assert.equal(handle.agent.session.snapshotEvents().findLast(e => e.type === 'turn/end').data.reason.kind, 'completed')
    const calls = (await f.calls()).filter(c => c.payload)
    assert(calls.every(c => Buffer.byteLength(JSON.stringify(c.payload)) <= 1024 * 1024))
    const prompt = calls.find(c => c.args[0] === 'session-init').payload.prompt
    const assistant = calls.find(c => c.payload.tool_name === 'assistant/message').payload
    const summary = calls.find(c => c.args[0] === 'summarize').payload
    assert(prompt.includes('remem_capture'))
    assert(summary.last_assistant_message.includes('remem_capture'))
    assert.equal(assistant.tool_input.remem_capture.omitted, true)
    assert.equal(assistant.tool_input.remem_capture.result_json_bytes, Buffer.byteLength(JSON.stringify({ text: body })))
    for (const value of [prompt, assistant.tool_response.text, summary.last_assistant_message]) {
      assert(value.includes('remem omitted remainder'))
      if (kind === 'multiline-escaped') assert(value.startsWith('Useful message evidence.'))
      else assert(!value.includes('private-token-'), 'omit overlong lines without cutting credential context')
    }
    assert.deepEqual(summary.reason, { kind: 'completed' })
  } finally {
    await handle?.dispose(); await fiber.dispose(); await ctx.fiber.dispose()
    await rm(f.root, { recursive: true, force: true })
  }
})

test('large turn-end metadata is bounded and its omission remains in captured text', async () => {
  const f = await fixture()
  const { ctx, fiber } = await harness(f.executable)
  try {
    const session = ctx.sessions.create(SessionId('large-turn-reason'), { meta: { cwd: tmpdir() } })
    session.append('turn/start', { turn: 1 })
    session.append('turn/end', { turn: 1, reason: { kind: 'error', error: {
      message: 'Failure detail.\n' + '证据"\\\n'.repeat(250000), code: 'UNKNOWN',
    } } })
    await ctx.parallel('session/flush', session)
    const summary = (await f.calls()).find(c => c.args[0] === 'summarize').payload
    assert(Buffer.byteLength(JSON.stringify(summary)) <= 1024 * 1024)
    assert.equal(summary.reason.kind, 'error')
    assert(summary.reason.error.message.startsWith('Failure detail.'))
    assert(summary.last_assistant_message.includes('remem_capture'))
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})


test('message envelopes preserve an exact JSON-byte fit and project one byte over', async () => {
  const f = await fixture()
  const { ctx, fiber } = await harness(f.executable)
  try {
    const session = ctx.sessions.create(SessionId('byte-boundary'), { meta: { cwd: tmpdir() } })
    const append = prompt => session.append('user/message', createUserMessage({
      content: [{ type: 'text', text: prompt }], source: { kind: 'user' },
    }), { surfaceOp: 'append' })
    append('probe')
    await ctx.parallel('session/flush', session)
    const sample = (await f.calls()).find(c => c.args[0] === 'session-init').payload
    const overhead = Buffer.byteLength(JSON.stringify({ ...sample, prompt: '' }))
    const escaped = '证😀"\\\t'.repeat(40000)
    const prompt = escaped + 'x'.repeat(1024 * 1024 - overhead - Buffer.byteLength(JSON.stringify(escaped)) + 2)
    append(prompt)
    append(prompt + 'x')
    await ctx.parallel('session/flush', session)
    const captures = (await f.calls()).filter(c => c.args[0] === 'session-init')
    assert.equal(Buffer.byteLength(JSON.stringify(captures[1].payload)), 1024 * 1024)
    assert.equal(captures[1].payload.prompt, prompt)
    assert(Buffer.byteLength(JSON.stringify(captures[2].payload)) <= 1024 * 1024)
    assert(captures[2].payload.prompt.includes('"original_envelope_bytes":1048577'))
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})


test('oversized tool identity fails visibly without renaming the tool', async () => {
  const f = await fixture()
  const { ctx, fiber } = await harness(f.executable)
  const errors = []
  ctx.logger.error = message => errors.push(message)
  try {
    const session = ctx.sessions.create(SessionId('large-tool-name'), { meta: { cwd: tmpdir() } })
    session.append('turn/start', { turn: 1 })
    session.append('step/start', { turn: 1, step: 1 })
    session.append('tool/call', { turn: 1, step: 1, callId: ToolCallId('large-name'),
      name: 't'.repeat(1024 * 1024), arguments: '{}' })
    session.append('tool/result', { turn: 1, step: 1, message: createToolResultMessage({
      callId: ToolCallId('large-name'), content: [{ type: 'text', text: 'external evidence' }], isError: false,
    }) }, { surfaceOp: 'append' })
    await assert.rejects(ctx.parallel('session/flush', session), error => error instanceof AggregateError
      && error.errors.some(e => e.message === 'remem: tool observation identity exceeds the stdin budget'))
    assert.deepEqual(errors, ['remem: tool observation identity exceeds the stdin budget'])
    assert(!(await f.calls()).some(c => c.args[0] === 'observe'))
  } finally { await fiber.dispose(); await ctx.fiber.dispose(); await rm(f.root, { recursive: true, force: true }) }
})
