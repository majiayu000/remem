# Candidate Auto-Promotion Product Contract

Status: Current contract
Date: 2026-08-03

Tracking:
- Implementation issue: #955
- Parent quality epic: #942

## Problem

The observation-path gate treated a list of common engineering words as an
all-or-nothing veto over the complete candidate. Words such as `not`, `fail`,
`skip`, `pass`, `will`, and `token` could block a supported repository fact
before its evidence was compared. The same gate rejected every `lesson`, so a
directly observed failure and recovery could not become durable memory.

The deterministic extraction corpus contained only two labeled cases and did
not expose risk-class distribution, unsupported claims, secrets, or instruction
injection boundaries.

## Decision

- Candidate extraction uses exactly three risk classes: `low`, `medium`, and
  `high`. Unknown values are parse errors, never aliases or defaults.
- Source support is evaluated per concrete sentence claim. Every claim must be
  supported by an eligible observation, and polarity/modality must match.
- Ordinary negative facts and engineering terminology do not fail because one
  token appears in a blacklist. Prospective, uncertain, conditional, and
  prescriptive claims remain review-gated even when their wording is repeated.
  The same applies to capability modals, outer/quoted meta-negation, imperative
  control text, authorization state, and affirmative destructive operations.
- Secret detection canonicalizes case, full-width ASCII, and punctuation or
  Unicode separators before checking credential-qualified token terms. Bare
  `token` in an explicit non-credential context such as `token budget` is not a
  secret; credential-qualified or ambiguous token usage remains blocked.
- A project-scoped, low-risk, high-confidence `lesson` may auto-promote only
  when both the lesson and an eligible bugfix/decision observation describe an
  affirmative failure linked to a recovery action, and all ordinary
  source/trust/routing gates pass. Failure or recovery keywords alone do not
  qualify.
- Preferences, procedures, session activity, unknown types, unsupported
  claims, external content, and instruction-pattern content remain fail-closed.

## User-Visible Behavior

- Supported failure lessons can enter active memory without manual review.
- Supported negative facts such as “does not delete active rows” can promote.
- Candidate rows that do not promote retain an explicit reason, including
  `lesson_not_failure_qualified`, `contains_unsafe_marker`,
  `claim_semantics_require_review`, or `no_supporting_source_observation`.
- Extraction eval output reports counts for each closed risk class.

## Acceptance

- At least 27 labeled deterministic extraction cases cover low/medium/high
  classification and adversarial unsupported, secret, and injection inputs.
- Claim-level tests prove all claims require evidence, while claims may draw
  support from different eligible observations.
- Negation/polarity (including outer quoted negation), future/capability modal
  language, secret variants, auth/authz or destructive semantics, and generic
  imperative prompt injection remain fail-closed.
- A supported failure lesson promotes; a non-failure lesson, preference, and
  procedure do not.
- Fresh formatter, check, clippy, full tests, and extraction eval pass.

Production promotion-rate and pending-review trend claims require a real-store
sample after deployment; deterministic fixtures do not stand in for that
operator evidence.
