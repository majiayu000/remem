# Retrieval Enrichment Budget

Status: Current contract
Date: 2026-08-13

Tracking:
- Local P0 incident: historical retrieval enrichment exhausted agent tokens after the v072 upgrade.
- Root behavior: GH-850 / v072 retrieval enrichment.

## Problem

The v072 migration made every historical memory immediately eligible for
AI-generated retrieval enrichment. The idle sweep processed 16 rows and told
the worker to continue whenever any row succeeded, so `remem worker --once`
could drain the entire historical table instead of doing one bounded unit of
idle work. A failed row also remained automatically retryable forever.

The usage ledger compounded the incident by applying the generic GPT-5 USD
fallback to GPT-5.6 Codex credit models such as Luna. That estimate is not the
billing unit used by those models and must not be presented as USD truth.

## Goals

- An upgrade never turns the complete historical memory table into automatic
  AI work.
- A once worker attempts at most one bounded retrieval-enrichment batch. A
  daemon attempts at most one batch every 60 seconds.
- A once worker processes at most four potential AI work items across current
  extraction tasks, AI-backed durable jobs, and retrieval enrichment; it
  also stops admitting new work after 180 seconds.
- New or canonically changed memories still receive automatic enrichment.
- A row stops automatic retries after three failed enrichment attempts.
- Historical and exhausted rows remain stored, searchable through their
  deterministic context, and visible in diagnostics.
- GPT-5.6 Codex credit models report unknown USD pricing unless an operator
  supplied an explicit USD override.

## User-Visible Behavior

- Migration marks incomplete pre-existing rows `deferred`; it does not delete
  them, rewrite canonical memory, or call AI.
- New rows start `pending`. Canonical title/content/type/topic/files changes
  reset the row to `pending` and clear prior retry state.
- Successful generation moves a row to `ready`. The third failed generation
  moves it to `exhausted`; automatic workers do not select `deferred` or
  `exhausted` rows.
- The idle batch size is four. A once worker can therefore admit no more than
  four separately queued potential-AI work items in total, even when multiple
  queues have backlog. An admitted surface retains its own bounded internal
  execution contract.
- Doctor output separates pending, exhausted, and deferred counts. Deferred
  historical rows do not cause a warning or tell the operator to repeatedly
  run the worker.
- Usage events for `gpt-5.6-luna`, `gpt-5.6-sol`, and `gpt-5.6-terra` keep
  token counts but use `estimated_cost_usd=0` and
  `pricing_source=unknown_pricing` unless an environment override supplies an
  explicit USD rate.

## Non-Goals

- No automatic backfill of rows marked `deferred`.
- No deletion or fake-ready marking of historical rows.
- No conversion from Codex credits to USD.
- No change to automatic capture or to canonical memory content.
- No promise that a task already inside one provider call is preempted by the
  process-level schedule.

## Acceptance Criteria

- Upgrading a v082 database marks incomplete existing rows `deferred`, keeps
  valid enriched rows `ready`, and makes post-upgrade inserts `pending`.
- Candidate selection and claim CAS reject `deferred` and `exhausted` rows.
- Canonical mutation resets a row to `pending`; success sets `ready`; the third
  failure sets `exhausted` with no next retry.
- A sweep with more than four due rows invokes the generator exactly four
  times, and a once-worker schedule admits no second sweep.
- Once-worker budget tests prove a four-work-item cap and a 180-second
  admission deadline; daemon mode remains long-running.
- A daemon schedule cannot admit a second sweep before 60 seconds.
- Static pricing returns unknown for GPT-5.6 Codex credit models, while an
  explicit operator pricing override still wins.
- Focused migration, enrichment, worker-schedule, doctor, and pricing tests
  pass, followed by `cargo fmt --check`, `cargo check`, and `cargo test`.
