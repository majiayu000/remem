import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'

// `remem install` has no DSH target and the adapter never creates the store,
// so this is the only step that gives a DSH-only machine a usable database.
const initialize = ['encrypt']
const documented = `remem ${initialize.join(' ')}`

test('README tells a DSH-only install how to create the remem store', async () => {
  const readme = await readFile(new URL('../README.md', import.meta.url), 'utf8')
  const [chinese, english] = readme.split('\n## English\n')
  assert(english, 'README must keep its English section')
  for (const [language, text] of [['Chinese', chinese], ['English', english]]) {
    assert(text.includes(`\`${documented}\``), `${language} install steps omit \`${documented}\``)
  }
})

test('documented initialization lets DSH capture reach a fresh encrypted store', {
  skip: !process.env.REMEM_DSH_BINARY,
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'dsh-remem-first-run-'))
  // Summarize queues background distillation; never let it reach a paid model.
  const executor = join(root, 'offline-codex')
  await writeFile(executor, `#!${process.execPath}\nprocess.exit(19)\n`, { mode: 0o700 })
  const config = join(root, 'config.toml')
  await writeFile(config, `[memory_ai.profiles.codex]\npath = ${JSON.stringify(executor)}\n`)
  // Default encryption: no plaintext override, no key supplied from outside.
  const env = { PATH: process.env.PATH, HOME: root, REMEM_DATA_DIR: join(root, 'data'), REMEM_CONFIG: config }
  const remem = (args, payload) => spawnSync(process.env.REMEM_DSH_BINARY, args,
    { env, input: payload === undefined ? undefined : JSON.stringify(payload), encoding: 'utf8', timeout: 30_000 })
  const base = { host: 'deepseek-harness', session_id: 'first-run', cwd: root,
    reference_time_epoch: Math.floor(Date.now() / 1000) }
  const captures = [
    ['session-init', { ...base, hook_event_name: 'UserPromptSubmit', turn_id: 'dsh:1', prompt: 'Remember: we use pnpm.' }],
    ['observe', { ...base, tool_name: 'assistant/message',
      tool_input: { dsh_event_type: 'assistant/message', arguments: {} }, tool_response: { text: 'Noted.' } }],
    ['summarize', { ...base, last_assistant_message: 'Noted.', reason: { kind: 'completed' } }],
  ]
  const context = () => remem(['context', '--host', 'deepseek-harness', '--cwd', root, '--session-id', 'first-run', '--gate', 'off'])

  for (const [command, payload] of captures) {
    assert.equal(remem([command, '--host', 'deepseek-harness'], payload).status, 1,
      `${command} must refuse a store that was never initialized`)
  }
  assert.match(context().stdout, /Context Load Errors/)

  const initialized = remem(initialize)
  assert.equal(initialized.status, 0, initialized.stderr)

  for (const [command, payload] of captures) {
    const run = remem([command, '--host', 'deepseek-harness'], payload)
    assert.equal(run.status, 0, `${command} failed after \`${documented}\`: ${run.stderr}`)
  }
  const ready = context()
  assert.equal(ready.status, 0, ready.stderr)
  assert.doesNotMatch(ready.stdout, /Context Load Errors/)
  // Background remem workers own their queues; leave this isolated scratch store for diagnostics.
  console.log(`Isolated first-run store: ${root}`)
})
