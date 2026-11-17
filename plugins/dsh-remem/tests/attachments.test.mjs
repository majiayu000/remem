import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { execFileSync } from 'node:child_process'
import { Context } from '@deepseek-ai/cordis'
import AgentRegistry from '@deepseek-ai/dsh-agent'
import { createUserMessage } from '@deepseek-ai/dsh-llm'
import SessionStore, { SessionId } from '@deepseek-ai/dsh-session'
const plugin = await import(process.env.REMEM_DSH_PLUGIN_MODULE ?? '../dist/index.js')

const image = { type: 'image', attachment: { attachmentId: 'diagram-ref', name: 'diagram.png',
  mediaType: 'image/png', bytes: 12345, width: 800, height: 600 } }
const imageReference = '[image attachment attachmentId=diagram-ref name=diagram.png mediaType=image/png bytes=12345 width=800 height=600]'

/** Commit human messages and flush their prompt capture; no agent turn is needed. */
async function capture(executable, sessionId, contents) {
  const ctx = new Context()
  await ctx.plugin(SessionStore)
  await ctx.plugin(AgentRegistry)
  const fiber = await ctx.plugin(plugin, { executable })
  try {
    const session = ctx.sessions.create(SessionId(sessionId), { meta: { cwd: tmpdir() } })
    for (const content of contents) {
      session.append('user/message', createUserMessage({ content, source: { kind: 'user' } }), { surfaceOp: 'append' })
    }
    await ctx.parallel('session/flush', session)
  } finally { await fiber.dispose(); await ctx.fiber.dispose() }
}

test('attachment fields stay short words in one bracketed reference per line', async () => {
  const root = await mkdtemp(join(tmpdir(), 'dsh-remem-attachments-'))
  try {
    const executable = join(root, 'remem')
    const log = join(root, 'prompts.jsonl')
    await writeFile(executable, `#!${process.execPath}\nimport fs from 'node:fs';\nlet data=''; for await (const c of process.stdin) data+=c;\nif (process.argv[2] === 'session-init') fs.appendFileSync(${JSON.stringify(log)}, JSON.stringify(JSON.parse(data).prompt)+'\\n');\n`, { mode: 0o700 })
    const notes = { type: 'file', attachment: { attachmentId: 'notes-ref', name: 'Q3 notes [final]\n"v2".pdf',
      mediaType: 'application/pdf', bytes: 48213 } }
    const bounded = { type: 'file', attachment: { attachmentId: 'data:text/plain;base64,SU5MSU5F', name: 'x'.repeat(600),
      mediaType: 'text/plain', bytes: 7, path: '/never/captured' } }
    const cut = { type: 'image', attachment: { name: `a${'😀'.repeat(300)}`, mediaType: 'image/png', width: 1, height: 1 } }
    const lone = { type: 'image', attachment: { name: 'odd\udc00.png' } }
    await capture(executable, 'attachment-format', [[image, notes], [{ type: 'text', text: 'mixed prompt' }, bounded, cut, lone]])
    const prompts = (await readFile(log, 'utf8')).trim().split('\n').map(JSON.parse)
    assert.deepEqual(prompts, [
      `${imageReference}\n[file attachment attachmentId=notes-ref name="Q3 notes [final]\\n\\"v2\\".pdf" mediaType=application/pdf bytes=48213]`,
      ['mixed prompt', `[file attachment name=${'x'.repeat(512)} mediaType=text/plain bytes=7]`,
        `[image attachment name=a${'😀'.repeat(255)} mediaType=image/png width=1 height=1]`,
        '[image attachment name="odd\\udc00.png"]'].join('\n'),
    ])
    // Core redaction masks whitespace-free words of 32+ characters that mix letters and digits.
    assert(prompts[0].split(/\s+/).every(word => word.length < 32))
  } finally { await rm(root, { recursive: true, force: true }) }
})

test('real remem persists attachment names, media types and sizes unmasked', {
  skip: !process.env.REMEM_DSH_BINARY,
}, async () => {
  const executable = process.env.REMEM_DSH_BINARY
  const root = await mkdtemp(join(tmpdir(), 'dsh-remem-attachments-live-'))
  const previous = Object.fromEntries(['HOME', 'REMEM_DATA_DIR', 'REMEM_CONFIG', 'REMEM_ALLOW_PLAINTEXT_DB'].map(key => [key, process.env[key]]))
  Object.assign(process.env, { HOME: root, REMEM_DATA_DIR: root, REMEM_CONFIG: join(root, 'config.toml'), REMEM_ALLOW_PLAINTEXT_DB: '1' })
  // Any queued extraction stays offline; this test asserts only the captured prompts.
  const executor = join(root, 'offline-executor.mjs')
  await writeFile(executor, `#!${process.execPath}\nimport fs from 'node:fs';let prompt='';for await(const c of process.stdin)prompt+=c;fs.writeFileSync(process.argv[process.argv.indexOf('--output-last-message')+1],prompt.includes('Extract durable observations')?JSON.stringify({no_observations:{reason:'attachment fixture'}}):'<summary>Attachment capture.</summary><structured_fields><request></request><decisions></decisions><learned></learned><next_steps></next_steps><preferences></preferences></structured_fields><segments></segments>');\n`, { mode: 0o700 })
  await writeFile(process.env.REMEM_CONFIG, `[memory_ai.profiles.codex]\npath = ${JSON.stringify(executor)}\n`)
  try {
    execFileSync(executable, ['cleanup', '--dry-run'], { stdio: 'ignore' })
    const notes = { type: 'file', attachment: { attachmentId: 'notes-ref', name: 'Q3 planning notes 2026-11-17.pdf',
      mediaType: 'application/pdf', bytes: 48213 } }
    // DSH ids are sha256 digests: core may mask that one word, but not the fields beside it.
    const photo = { type: 'image', attachment: { attachmentId: `sha256:${'0123456789abcdef'.repeat(4)}`, name: 'whiteboard.jpg',
      mediaType: 'image/jpeg', bytes: 204800, width: 1600, height: 1200 } }
    await capture(executable, 'live-attachment-metadata', [[image, notes], [{ type: 'text', text: 'Compare the whiteboard photo.' }, photo]])
    const rows = JSON.parse(execFileSync('/usr/bin/python3', ['-c', `import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1]); print(json.dumps([r[0] for r in c.execute("SELECT content_text FROM captured_events WHERE session_id=? AND event_type='user_prompt_submit' ORDER BY id",(sys.argv[2],))]))`, join(root, 'remem.db'), 'live-attachment-metadata'], { encoding: 'utf8' }))
    assert.deepEqual(rows, [
      `${imageReference}\n[file attachment attachmentId=notes-ref name="Q3 planning notes 2026-11-17.pdf" mediaType=application/pdf bytes=48213]`,
      'Compare the whiteboard photo.\n[image attachment [REDACTED] name=whiteboard.jpg mediaType=image/jpeg bytes=204800 width=1600 height=1200]',
    ])
  } finally {
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[key]; else process.env[key] = value
    }
    // Background remem workers may still own this isolated store; keep it for diagnostics.
    console.log(`Isolated attachment store: ${root}`)
  }
})
