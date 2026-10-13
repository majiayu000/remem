import { spawn } from 'node:child_process'
import { isAbsolute, join } from 'node:path'
import type { Context } from '@deepseek-ai/cordis'
import type { Agent, PreStepDecision } from '@deepseek-ai/dsh-agent'
import { createDeveloperMessage, createUserMessage } from '@deepseek-ai/dsh-llm'
import type { UserMessage } from '@deepseek-ai/dsh-llm'
import type { Session, SessionEvent, SessionSeq } from '@deepseek-ai/dsh-session'

declare module '@deepseek-ai/dsh-llm' {
  interface MessageSourceMap {
    remem: { kind: 'remem'; form: 'snapshot'; sections: readonly { name: string; text: string }[] }
  }
}

export const name = 'remem'
export const inject = ['agents', 'sessions']

export interface Config {
  /** An explicitly installed remem executable; never downloaded by this plugin. */
  executable?: string
  /** Existing remem memory_ai profile used for turn summaries. */
  profile?: string
}

const host = 'deepseek-harness'
const timeoutMs = 30_000
const maxOutputBytes = 1024 * 1024
const maxObservationBytes = 1024 * 1024

const jsonBytes = (value: unknown) => Buffer.byteLength(JSON.stringify(value) ?? 'null')

/** Retain structure and complete text lines so core redaction keeps its key/token context. */
function projectEvidence(value: unknown, budget: number): unknown {
  const originalBytes = jsonBytes(value)
  if (originalBytes <= budget) return value
  if (typeof value === 'string') {
    const marker = `\n[remem omitted remainder; original JSON bytes: ${originalBytes}]`
    let low = 0, high = Math.min(value.length, budget)
    while (low < high) {
      const mid = Math.ceil((low + high) / 2)
      if (jsonBytes(value.slice(0, mid) + marker) <= budget) low = mid
      else high = mid - 1
    }
    // Never cut a credential/token or assignment in half before core redaction.
    const end = value.lastIndexOf('\n', low - 1)
    return (end < 0 ? '' : value.slice(0, end)) + marker
  }
  const array = Array.isArray(value)
  const entries = Object.entries(value as object)
  const kept: [string, unknown][] = []
  let remaining = budget - 2
  for (const [key, child] of entries) {
    const overhead = (kept.length ? 1 : 0) + (array ? 0 : jsonBytes(key) + 1)
    const allowance = Math.min(remaining - overhead, Math.floor(budget / 2))
    if (allowance < 96) break
    const projected = projectEvidence(child, allowance)
    kept.push([key, projected])
    remaining -= overhead + jsonBytes(projected)
  }
  return array ? kept.map(([, child]) => child) : Object.fromEntries(kept)
}

function toolObservation(base: object, name: string, args: unknown, result: unknown): unknown {
  const payload = { ...base, tool_name: name, tool_input: { arguments: args }, tool_response: result }
  const originalBytes = jsonBytes(payload)
  if (originalBytes <= maxObservationBytes) return payload
  // This sidecar is inside tool_input so the existing core ledger retains it.
  const remem_capture = { omitted: true, original_envelope_bytes: originalBytes,
    arguments_json_bytes: jsonBytes(args), result_json_bytes: jsonBytes(result),
    policy: 'bounded structure and complete text lines; remaining entries/lines omitted' }
  const fixed = { ...base, tool_name: name, tool_input: { arguments: null, remem_capture }, tool_response: null }
  const available = maxObservationBytes - jsonBytes(fixed)
  if (available < 512) throw new Error('remem: tool observation identity exceeds the stdin budget')
  return { ...fixed,
    tool_input: { arguments: projectEvidence(args, Math.floor(available / 4)), remem_capture },
    tool_response: projectEvidence(result, Math.floor(available * 3 / 4)),
  }
}

/** Spawn argv without a shell. Do not copy potentially secret-bearing stderr to host logs. */
async function run(executable: string, args: string[], input?: unknown, signal?: AbortSignal): Promise<string> {
  signal?.throwIfAborted()
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, { detached: process.platform !== 'win32',
      stdio: [input === undefined ? 'ignore' : 'pipe', 'pipe', 'pipe'] })
    const chunks: Buffer[] = []
    let size = 0
    let failure: Error | undefined
    let stopping: Promise<void> | undefined
    const abort = () => {
      if (stopping || child.pid === undefined) return
      stopping = new Promise<void>((stopped, stopFailed) => {
        const fail = () => stopFailed(new Error(`remem ${args[0]} could not stop its process tree`))
        if (process.platform === 'win32') {
          const systemRoot = process.env.SystemRoot
          if (!systemRoot || !isAbsolute(systemRoot)) { fail(); return }
          const killer = spawn(join(systemRoot, 'System32', 'taskkill.exe'), ['/PID', String(child.pid), '/T', '/F'],
            { stdio: 'ignore', windowsHide: true })
          killer.on('error', fail)
          killer.on('close', code => {
            if (code === 0) stopped()
            else fail()
          })
        } else {
          try { process.kill(-child.pid!, 'SIGKILL'); stopped() }
          catch (error) {
            if ((error as NodeJS.ErrnoException).code === 'ESRCH') stopped()
            else fail()
          }
        }
      })
      stopping.catch(error => { cleanup(); reject(error) })
    }
    signal?.addEventListener('abort', abort, { once: true })
    const cleanup = () => {
      clearTimeout(timer)
      signal?.removeEventListener('abort', abort)
    }
    const timer = setTimeout(() => {
      failure = new Error(`remem ${args[0]} timed out after ${timeoutMs}ms`)
      abort()
    }, timeoutMs)
    child.stdout!.on('data', (chunk: Buffer) => {
      size += chunk.length
      if (size > maxOutputBytes) {
        failure = new Error(`remem ${args[0]} exceeded the output limit`)
        abort()
      } else chunks.push(chunk)
    })
    child.stderr!.resume()
    child.stdin?.on('error', error => { failure ??= error })
    child.on('error', () => {
      cleanup()
      reject(new Error('Cannot start remem; install @remem-ai/remem or set executable to its absolute path'))
    })
    child.on('close', async (code, childSignal) => {
      // The stop failure already rejected this run through stopping.catch.
      try { await stopping } catch { return }
      cleanup()
      if (signal?.aborted) reject(signal.reason)
      else if (failure) reject(failure)
      else if (code !== 0) reject(new Error(`remem ${args[0]} failed (exit ${code}, signal ${childSignal ?? 'none'}); inspect the remem log`))
      else resolve(Buffer.concat(chunks).toString('utf8'))
    })
    if (input !== undefined) child.stdin?.end(JSON.stringify(input))
  })
}

function cwd(session: Session): string {
  if (!session.header.cwd) throw new Error(`remem: DSH session ${session.id} has no project cwd`)
  return session.header.cwd
}

function text(content: readonly unknown[], attachments = false): string {
  return content.flatMap(block => {
    if (typeof block === 'object' && block !== null && 'type' in block && block.type === 'text'
      && 'text' in block && typeof block.text === 'string') return [block.text]
    if (attachments && typeof block === 'object' && block !== null && 'type' in block
      && (block.type === 'image' || block.type === 'file')) {
      const ref = 'attachment' in block ? block.attachment : undefined
      const metadata: Record<string, string | number> = {}
      if (typeof ref === 'object' && ref !== null) {
        for (const key of ['attachmentId', 'name', 'mediaType', 'bytes', 'width', 'height'] as const) {
          if (!(key in ref)) continue
          const value = (ref as Record<string, unknown>)[key]
          if (typeof value === 'string' && !value.startsWith('data:')) metadata[key] = value.slice(0, 512)
          else if (typeof value === 'number' && Number.isFinite(value)) metadata[key] = value
        }
      }
      return [`[${block.type} attachment ${JSON.stringify(metadata)}]`]
    }
    return []
  }).join('\n')
}

export async function apply(ctx: Context, config: Config = {}): Promise<void> {
  const executable = config.executable ?? 'remem'
  const lifetime = new AbortController()
  const contextRuns = new Map<Promise<string>, AbortSignal>()
  // Availability only. The first real CLI request validates the DSH host contract.
  await run(executable, ['--version'])
  const states = new Map<Session, { pending: Promise<void>; error?: Error;
    admission?: { turn: number; step: number; signal: AbortSignal; messages: UserMessage[]; replacements: SessionSeq[] };
    calls: Map<string, { name: string; arguments: string }>; lastAnswer?: string }>()
  const state = (session: Session) => {
    let value = states.get(session)
    if (!value) { value = { pending: Promise.resolve(), calls: new Map() }; states.set(session, value) }
    return value
  }
  const recoverAdmission = (agent: Agent, value: ReturnType<typeof state>, turn: number) => {
    const admission = value.admission
    value.admission = undefined
    if (admission?.turn !== turn || admission.signal.aborted) return
    // Cordis may already have removed our post-commit listener during unload.
    const committed = new Set(agent.session.snapshotEvents()
      .filter(event => event.type === 'user/message').map(event => event.data.id))
    const messages = admission.messages.filter(message => !committed.has(message.id))
    if (messages.length) agent.inbox.splice('next-step', 0, 0, messages)
  }
  const wait = async (session: Session, signal?: AbortSignal) => {
    const value = state(session)
    let onAbort: (() => void) | undefined
    try {
      signal?.throwIfAborted()
      await (signal ? Promise.race([value.pending, new Promise<never>((_, reject) => {
        onAbort = () => reject(signal.reason)
        signal.addEventListener('abort', onAbort, { once: true })
      })]) : value.pending)
    } finally {
      if (onAbort) signal!.removeEventListener('abort', onAbort)
    }
    signal?.throwIfAborted()
    // A cancelled preparation leaves capture and its error for a later flush.
    const error = value.error
    value.error = undefined
    if (error) throw error
  }
  const capture = (session: Session, command: string, payload: unknown) => {
    const value = state(session)
    // Keep processing later events after failure; retain the loss until flush/step reports it.
    value.pending = value.pending.then(async () => {
      try {
        const args = [command, '--host', host]
        if (command === 'summarize' && config.profile) args.push('--profile', config.profile)
        await run(executable, args, payload)
      } catch (error) {
        value.error ??= error instanceof Error ? error : new Error(String(error))
        ctx.logger.error(value.error.message)
      }
    })
  }

  ctx.on('agent/pre-step', async ({ agent, messages, turn, step, signal }, next): Promise<PreStepDecision> => {
    const value = state(agent.session)
    try {
      const decision = await next()
      if (decision.kind === 'reject' || signal.aborted) return decision
      // DSH completes an empty first-step admission without a model request.
      // Preserve that downstream decision instead of creating a memory-only turn.
      if (step === 1 && decision.messages.length === 0) return decision
      let memory: string
      const contextSignal = AbortSignal.any([signal, lifetime.signal])
      contextSignal.throwIfAborted()
      await wait(agent.session, contextSignal)
      contextSignal.throwIfAborted()
      value.admission = { turn, step, signal, messages: [...messages], replacements: [] }
      if (step !== 1) return decision
      const lookup = run(executable, ['context', '--host', host, '--cwd', cwd(agent.session),
        '--session-id', agent.session.id, '--gate', 'off'], undefined, contextSignal)
      contextRuns.set(lookup, contextSignal)
      try { memory = await lookup } finally { contextRuns.delete(lookup) }
      contextSignal.throwIfAborted()
      const session = agent.session
      const snapshots = session.snapshotEvents().filter(event => session.surface.nodes.includes(event.seq)
        && event.type === 'user/message' && event.data.source.kind === 'remem')
      const previous = session.deriveMessages().findLast(message => message.source.kind === 'remem')
      const unchanged = previous?.source.kind === 'remem' && previous.source.sections.length === 1
        && previous.source.sections[0].name === 'memory' && previous.source.sections[0].text === memory
      // Pre-step runs before step/start: prepare replacements without appending.
      value.admission.replacements = (unchanged ? snapshots.slice(0, -1) : snapshots).map(event => event.seq)
      if (!memory.trim() || unchanged) return decision
      return { ...decision, messages: [...decision.messages, createUserMessage({
        content: [{ type: 'text', text: memory }],
        source: { kind: 'remem', form: 'snapshot', sections: [{ name: 'memory', text: memory }] },
      })] }
    } catch (error) {
      value.admission = undefined
      // DSH has already claimed these original inputs. Park them for an explicit
      // retry before propagating failure; user cancellation still discards work.
      if (!signal.aborted) agent.inbox.splice('next-step', 0, 0, messages)
      throw error
    }
  }, { prepend: true })

  ctx.on('agent/request', async ({ agent, turn, step, signal }, next) => {
    // Hold the state across next(): disposal clears the map and removes agent/error.
    const value = state(agent.session)
    try {
      const config = await next()
      AbortSignal.any([signal, lifetime.signal]).throwIfAborted()
      const admission = value.admission
      if (admission?.turn === turn && admission.step === step) {
        // This awaited lifecycle runs inside the open step. Native replacements
        // preserve provenance and omit obsolete snapshots from derived requests.
        for (const seq of admission.replacements) {
          agent.session.append('developer/message', { turn, step, message: createDeveloperMessage({ content: [],
            source: { kind: 'remem', form: 'snapshot', sections: [] } }) }, {
            surfaceOp: { op: 'replace', startSeq: seq, endSeq: seq }, sourceEventSeqs: [seq],
          })
        }
        admission.replacements = []
      }
      return config
    } catch (error) {
      recoverAdmission(agent, value, turn)
      throw error
    }
  }, { prepend: true })

  ctx.on('agent/error', ({ agent, turn }) => {
    // This notification is outside session append publication, so inbox edits
    // are legal here. Only originals not yet committed need an explicit retry.
    recoverAdmission(agent, state(agent.session), turn)
  })

  ctx.on('session/event', (session, event: SessionEvent) => {
    try {
      const base = { host, session_id: session.id, cwd: cwd(session), reference_time_epoch: Math.floor(event.time / 1000) }
      const value = state(session)
      if (event.type === 'user/message' && value.admission) {
        value.admission.messages = value.admission.messages.filter(message => message.id !== event.data.id)
      }
      if (event.type === 'turn/start') { value.lastAnswer = undefined; return }
      if (event.type === 'tool/call') { value.calls.set(event.data.callId, event.data); return }
      if (event.type === 'user/message' && event.data.source.kind === 'user') {
        capture(session, 'session-init', { ...base, hook_event_name: 'UserPromptSubmit',
          prompt: text(event.data.content, true), turn_id: `dsh:${event.seq}` })
      } else if (event.type === 'assistant/message') {
        const answer = text(event.data.message.content)
        if (answer) value.lastAnswer = answer
        if (answer) capture(session, 'observe', { ...base, tool_name: 'assistant/message', tool_response: { text: answer } })
      } else if (event.type === 'tool/result') {
        const result = event.data.message
        const call = value.calls.get(result.toolCallId)
        if (!call) throw new Error(`remem: DSH tool result ${result.toolCallId} has no live call`)
        value.calls.delete(result.toolCallId)
        let argumentsValue: unknown
        try { argumentsValue = call.arguments === '' ? {} : JSON.parse(call.arguments) }
        catch { throw new Error('remem: DSH tool arguments are invalid JSON; capture rejected') }
        capture(session, 'observe', toolObservation(base, call.name, argumentsValue, event.data))
      } else if (event.type === 'turn/end') {
        value.admission = undefined
        capture(session, 'summarize', { ...base, last_assistant_message: value.lastAnswer, reason: event.data.reason })
      }
    } catch (error) {
      const value = state(session)
      value.error ??= error instanceof Error ? error : new Error(String(error))
      ctx.logger.error(value.error.message)
    }
  })
  ctx.on('session/flush', wait)
  ctx.on('session/disposed', async session => {
    try { await wait(session) } finally { states.delete(session) }
  })
  ctx.effect(() => async () => {
    lifetime.abort(new Error('remem plugin disposed'))
    const contexts = [...contextRuns]
    const [settled, stopped] = await Promise.all([
      Promise.allSettled([...states.keys()].map(session => wait(session))),
      Promise.allSettled(contexts.map(([lookup]) => lookup)),
    ])
    states.clear()
    const errors = settled.flatMap(result => result.status === 'rejected' ? [result.reason] : [])
    for (const [index, result] of stopped.entries()) {
      if (result.status === 'rejected' && result.reason !== contexts[index][1].reason) errors.push(result.reason)
    }
    if (errors.length) throw new AggregateError(errors, 'remem capture failed during disposal')
  })
}
