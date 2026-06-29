# Codex App SessionStart Visibility Boundary

Date: 2026-06-29

## Status

Remem can inject model-visible Codex App context through the Codex `SessionStart`
hook, but current Codex App builds do not render that context as a visible block
inside the conversation transcript.

The local Remem app/HUD is useful for diagnostics and plugin app testing, but it
is not the same product surface as an in-thread Codex App context card. Do not
claim in-thread visibility from the HUD alone.

## Observed Environment

- Codex App bundle: `/Applications/Codex.app`
- App version observed: `26.623.61825`
- Bundled CLI observed: `codex-cli 0.142.3`
- App server schema includes `ThreadItem` variants such as `userMessage`,
  `agentMessage`, and `hookPrompt`.
- Hook run entries support `warning`, `stop`, `feedback`, `context`, and
  `error` kinds.

These details can drift with Codex App releases. Recheck the installed App
bundle and generated app-server schema before implementing against this note.

## Current Runtime Path

For a Remem Codex `SessionStart` hook:

1. The hook emits Codex hook JSON.
2. `hookSpecificOutput.additionalContext` becomes model-visible hidden context.
3. `systemMessage` becomes a hook warning/status entry.
4. Codex App receives `hook/started` and `hook/completed` notifications.
5. The App stores hook run state and can show hook stats/tooltip UI.
6. No `ThreadItem::HookPrompt` is produced for `SessionStart`.
7. Because the conversation renderer is item-based, no in-thread context block
   appears.

This explains the observed behavior: the model can use Remem context, but the
current Codex App conversation view does not show a CLI-style startup block.

## Existing Visible Path

Codex already has one visible hook-feedback path:

1. A `Stop` hook returns `decision:block` with a non-empty `reason`.
2. Codex converts that reason into `HookPromptFragment`.
3. Core records a `hook_prompt` response item.
4. App server converts that response item into `ThreadItem::HookPrompt`.
5. Codex App renders it as a visible hook-feedback user message.

This path is not suitable for Remem startup visibility. It semantically blocks a
turn and forces a continuation pass; using it for startup status would change
agent behavior just to get UI.

## Why Remem Alone Cannot Fix This

Remem can change its hook output, but Codex App only renders typed thread items
it receives from Codex core/app-server. Today, `SessionStart` context output is
not mapped to a visible thread item.

Changing only Remem can:

- keep full context hidden through `additionalContext`;
- provide hook metadata through `systemMessage`;
- update the local Remem HUD/status file;
- expose plugin app or connector metadata.

Changing only Remem cannot:

- create an in-thread Codex App context card from `SessionStart`;
- make `additionalContext` visible in the conversation transcript;
- make the App render arbitrary plugin HTML inside the current thread.

## Product Requirement

The target in Codex App is a small, internal, in-thread Remem status block at
session start:

- status: loaded, suppressed, or error;
- project path and branch;
- updated timestamp;
- memory/preference/session/workstream counts;
- short note that the full payload is hidden model context;
- optional retrieval hint such as `search` and `get_observations`.

The block should be visible in the current Codex App conversation, not in an
external browser page or detached HUD.

## Preferred Upstream Shape

Codex core should add a first-class way for context-injecting hooks to emit a
visible startup summary. Two reasonable designs:

1. Reuse `ThreadItem::HookPrompt` for non-blocking startup summaries.
2. Add a new item, for example `ThreadItem::HookContextStatus`, with explicit
   metadata and rendering.

The minimal compatibility path is to reuse `HookPrompt`, but the cleaner design
is a dedicated context-status item because startup visibility is not feedback
for a blocked turn.

## Required Codex Changes

An upstream Codex implementation should include:

- hook schema support for an optional visible context/status field on
  `SessionStart` and other context-injecting hooks;
- `SessionStartOutcome` carrying both hidden `additionalContext` and visible
  summary fragments;
- core recording a visible response/thread item after `SessionStart` succeeds;
- app-server live notification and thread-history rebuild support;
- App renderer support if a new item type is added;
- tests proving hidden model context and visible UI metadata remain separate.

## Required Remem Changes After Upstream Support

After Codex exposes the visible startup contract, Remem should:

- emit full memory payload only through `additionalContext`;
- emit a short visible summary through the new visible field;
- update plugin README and skill docs to distinguish hidden context, hook
  summary metadata, local HUD, and in-thread App visibility;
- add wrapper tests for the visible summary payload;
- keep strict duplicate suppression so repeated `SessionStart` calls do not
  spam the conversation.

## Non-Solutions

- External web HUD: useful for diagnostics, but not in-thread Codex App UI.
- Patching `/Applications/Codex.app` or `app.asar`: useful only for local
  experiments; brittle under signature checks and app updates.
- Stop-hook block workaround: visible but semantically wrong and can trigger
  extra model passes.
- Assistant self-reporting memory state: pollutes model output and is not hook
  UI.
- Treating `systemMessage` as a visible context block: current App behavior does
  not support that interpretation.

## Commit Boundaries

Suggested commit split:

1. Documentation correction: capture this boundary and remove claims that
   Remem-only `systemMessage` creates in-thread Codex App visibility.
2. Upstream Codex change: add the visible `SessionStart` hook item contract.
3. Remem follow-up: emit the new visible summary field and update tests.
4. Optional App UX polish: richer rendering for the dedicated context-status
   item if Codex chooses a new `ThreadItem` variant.
