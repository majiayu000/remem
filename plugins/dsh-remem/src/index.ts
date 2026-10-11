import { spawn } from 'node:child_process'
import { isAbsolute, join } from 'node:path'
import type { Context } from '@deepseek-ai/cordis'
import type { PreStepDecision } from '@deepseek-ai/dsh-agent'
import { createDeveloperMessage, createUserMessage } from '@deepseek-ai/dsh-llm'
import type { Session, SessionEvent } from '@deepseek-ai/dsh-session'

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
    calls: Map<string, { name: string; arguments: string }>; lastAnswer?: string }>()
  const state = (session: Session) => {
    let value = states.get(session)
    if (!value) { value = { pending: Promise.resolve(), calls: new Map() }; states.set(session, value) }
    return value
  }
  const wait = async (session: Session) => {
    const value = state(session)
    await value.pending
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
    const decision = await next()
    if (decision.kind === 'reject' || signal.aborted) return decision
    // DSH completes an empty first-step admission without a model request.
    // Preserve that downstream decision instead of creating a memory-only turn.
    if (step === 1 && decision.messages.length === 0) return decision
    let memory: string
    const contextSignal = AbortSignal.any([signal, lifetime.signal])
    try {
      contextSignal.throwIfAborted()
      await wait(agent.session)
      contextSignal.throwIfAborted()
      if (step !== 1) return decision
      const lookup = run(executable, ['context', '--host', host, '--cwd', cwd(agent.session),
        '--session-id', agent.session.id, '--gate', 'off'], undefined, contextSignal)
      contextRuns.set(lookup, contextSignal)
      try { memory = await lookup } finally { contextRuns.delete(lookup) }
      contextSignal.throwIfAborted()
    } catch (error) {
      // DSH has already claimed these original inputs. Park them for an explicit
      // retry before propagating failure; user cancellation still discards work.
      if (!signal.aborted) agent.inbox.splice('next-step', 0, 0, messages)
      throw error
    }
    if (signal.aborted) return decision
    const session = agent.session
    const snapshots = session.snapshotEvents().filter(event => session.surface.nodes.includes(event.seq)
      && event.type === 'user/message' && event.data.source.kind === 'remem')
    const previous = session.deriveMessages().findLast(message => message.source.kind === 'remem')
    const unchanged = previous?.source.kind === 'remem' && previous.source.sections.length === 1
      && previous.source.sections[0].name === 'memory' && previous.source.sections[0].text === memory
    // Native replacement retains the original log event and its provenance.
    // Empty developer nodes are omitted by DSH's durable request derivation.
    // No new snapshot is committed here: cancelled admission can still retry.
    for (const event of unchanged ? snapshots.slice(0, -1) : snapshots) {
      session.append('developer/message', { turn, step, message: createDeveloperMessage({ content: [],
        source: { kind: 'remem', form: 'snapshot', sections: [] } }) }, {
        surfaceOp: { op: 'replace', startSeq: event.seq, endSeq: event.seq }, sourceEventSeqs: [event.seq],
      })
    }
    if (!memory.trim() || unchanged) return decision
    return { ...decision, messages: [...decision.messages, createUserMessage({
      content: [{ type: 'text', text: memory }],
      source: { kind: 'remem', form: 'snapshot', sections: [{ name: 'memory', text: memory }] },
    })] }
  }, { prepend: true })

  ctx.on('session/event', (session, event: SessionEvent) => {
    try {
      const base = { host, session_id: session.id, cwd: cwd(session), reference_time_epoch: Math.floor(event.time / 1000) }
      const value = state(session)
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
        try { argumentsValue = JSON.parse(call.arguments) }
        catch { throw new Error('remem: DSH tool arguments are invalid JSON; capture rejected') }
        capture(session, 'observe', { ...base, tool_name: call.name,
          tool_input: { arguments: argumentsValue }, tool_response: event.data })
      } else if (event.type === 'turn/end') {
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
      Promise.allSettled([...states.keys()].map(wait)),
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
