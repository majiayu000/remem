# GH969 Repository-Local Close Audit

Status: Complete for the repository-local stabilization contract; #931, #933,
and #935 remain independent parked contracts.

Audit date: 2026-09-03

Audit base: `d2072f1e` (post-v0.6.86 main)

Architecture/spec reconciliation implementation: #1050, `b800af4f`

## Verdict

The repository-local work promised by #969 is complete. The audit does not
claim that every planned research program or breaking follow-up has shipped.
Experimental and spec-only work remains gated by its own contract, while the
parked #931, #933, and #935 issues remain open and independent.

Two groups of old checklist wording are explicitly amended:

1. The largest cyclic component has not shrunk. It is 37 roots at both the
   #1045 baseline and this audit base. Closure therefore uses the shipped
   no-expansion, visible-edge, shrink-only-baseline contract. No reduction is
   claimed and no unrelated crate split is introduced.
2. Experimental Router/Graph/vector/default work is not declared production
   merely to close this epic. Existing production surfaces retain their
   evidence and rollback contracts; incomplete surfaces remain experimental or
   spec-only and cannot become default without same-head ablation and stop-loss
   evidence.

## Evidence Keys

| Key | Landed implementation | Primary repository evidence |
|---|---|---|
| C0 | GH969 contract, PR #1039, `886e6eda` | `PRODUCT.md`, `TECH.md`, specs index |
| S0 | Dream boundary, PR #987, `044b0d04`; backfill PR #996, `a923d4db` | Dream poisoning, provenance, migration, exposure, and review tests |
| S1 | Production-path security eval, PR #998, `6836327d` | `production_pipeline.rs`, persisted snapshots, adversarial-policy-v2 report |
| S2 | Legacy event projection, PR #999, `99777754` | projection/migration retry and idempotency tests |
| S3 | CurrentTruth identity isolation, PR #1038, `86fee409` | `current_truth_activation` relation-isolation regressions |
| G0 | Activation boundary, PR #1041, `80acc689` | activation receipt/replay tests and `check_active_memory_writes.py` |
| G1 | Surface lifecycle, PR #1043, `5b98e80d` | `surface-manifest.json` and `check_public_surface.py` |
| G2 | Dependency direction, PR #1045, `36f3d38c` | reviewed baseline, scanner/self-tests, current size 37 |
| G3 | Ship matrix, PR #1052, `257fc4a0` | `eval-gates`, authority verdict, matrix/scorecard tests |
| G4 | Documentation guard, PR #1057, `286326e0` | local-link/bilingual documentation checks |
| G5 | Exact-main native evidence, PR #1062, `36f06d7a` | four-platform security artifacts and successful main CI |
| G6 | Architecture/current-spec reconciliation, #1050, `b800af4f`; review hardening `5a9a98d1` | architecture lifecycle map, affirmative canonical-spec handoff check/tests |

The lifecycle/date and rollback columns below point to the canonical surface
inventory and the Product contract's migration rules. “Independent” means the
item has an explicit owner and contract outside this epic; it is not hidden
work and is not counted as completion evidence for #969.

## Original Checklist Mapping

### A. Current PRODUCT/TECH stabilization contract

| ID | Original criterion | Verdict | Implementation / exact SHA | Contract and regression evidence | Lifecycle, rollback, dependency |
|---|---|---|---|---|---|
| A1 | Current PRODUCT/TECH pair and index | Satisfied | C0 `886e6eda`; G6 `b800af4f` | `PRODUCT.md`, `TECH.md`, `docs/specs/README.md` | Current; rollback by reverting contract change; none |
| A2 | Single active-memory boundary and trust/review rules | Satisfied | G0 `80acc689` | Product “Active-Memory Safety Boundary”; activation bypass guard/tests | Production, continuous; route-specific rollback only; none |
| A3 | Lifecycle vocabulary and exit conditions | Satisfied | G1 `5b98e80d` | Product “Lifecycle Vocabulary”; manifest consistency guard | Per-row dates in manifest; rollback recorded per surface; independent owners retained |
| A4 | Separate merge/release/default/public-claim gates | Satisfied | G3 `257fc4a0` | Product “Decision Gates”; Technical “Unified Ship Matrix” | Fail closed; runtime verdict is authoritative; release authorization remains external |
| A5 | Migration, compatibility, rollback | Satisfied | C0 `886e6eda`; G0 `80acc689` | Product “Migration And Rollback Principles”; Technical migration section | Current; migration-specific rollback; none |

### B. Security and data-integrity closure

| ID | Original criterion | Verdict | Implementation / exact SHA | Contract and regression evidence | Lifecycle, rollback, dependency |
|---|---|---|---|---|---|
| B1 | Scan every Dream-generated field before write | Satisfied | S0 `044b0d04` | Dream poisoning and generated-surface regression suites | Production; quarantine is fail-closed; none |
| B2 | Risky Dream output is quarantined and cannot supersede | Satisfied | S0 `044b0d04` | Dream exposure, atomicity, review-policy tests | Production; operator review/backfill path; none |
| B3 | Record model trust, operation, provenance | Satisfied | S0 `044b0d04` | Dream provenance schema and review regressions | Production; immutable provenance retained; none |
| B4 | Project/owner-aware external candidate identity | Satisfied | S0 `044b0d04` | `external_identity` route and migration invariants | Production; migration/backfill documented; none |
| B5 | Same native topic remains isolated by project | Satisfied | S0 `044b0d04` | Claude native import and candidate identity regressions | Production; no compatibility fallback; none |
| B6 | Legacy events projection is idempotent/transactional | Satisfied | S2 `99777754` | hook/Cursor projection and partial-failure retry tests | Deprecated projection; preserve readers; review 2026-11-30 |
| B7 | CurrentTruth relations cannot cross scope/owner | Satisfied | S3 `86fee409` | CurrentTruth activation and cross-subject isolation tests | v1 production; breaking v2 remains parked in #933 |

### C. Real production-path security eval

| ID | Original criterion | Verdict | Implementation / exact SHA | Contract and regression evidence | Lifecycle, rollback, dependency |
|---|---|---|---|---|---|
| C1 | Label scanner-only evidence accurately | Satisfied | S1 `6836327d` | adversarial-policy report schema and docs | Production eval; public wording governed by G3 |
| C2 | Exercise capture through context in a real DB | Satisfied | S1 `6836327d` | `production_pipeline.rs` and SQLite snapshots | Deterministic eval production surface; none |
| C3 | Derive counts from persisted state | Satisfied | S1 `6836327d`; G3 `257fc4a0` | snapshot verifier and authority verdict tests | Runtime-only verdict; stale evidence fails closed |
| C4 | Share production surface-aware scanner contract | Satisfied | S1 `6836327d` | memory-bench production pipeline and poisoning tests | Production; no synthetic-count fallback |
| C5 | Cover adversarial and benign fixture classes | Satisfied | S1 `6836327d`; G5 `36f06d7a` | EN/ZH, authority, concealment, opaque, quoted, Dream artifacts | Four-platform evidence; exact-head identity required |
| C6 | Report FP/FN/leak/visibility outcomes | Satisfied | S1 `6836327d`; G3 `257fc4a0` | verified report plus outcome scorecard | Public claims remain false until their row passes |

### D. Experimental surface governance

| ID | Original criterion | Verdict | Implementation / exact SHA | Contract and regression evidence | Lifecycle, rollback, dependency |
|---|---|---|---|---|---|
| D1 | Canonical inventory covers major surfaces | Satisfied | G1 `5b98e80d` | Product inventory and `surface-manifest.json` | Machine checked; each row has an owner/date |
| D2 | Owner/status/caller/default/eval/date per surface | Satisfied | G1 `5b98e80d` | `check_public_surface.py` rejects omissions/drift | Rollback and compatibility are row-specific |
| D3 | Unintegrated modules are private or explicit experimental | Satisfied | G1 `5b98e80d` | caller/default classifications and source scans | Experimental surfaces cannot imply production |
| D4 | #932/#933/#934 need evidence and stop-loss before integration | Satisfied as gate | G1 `5b98e80d`; G3 `257fc4a0` | default-on matrix plus GH932/GH933/GH934 contracts | #933 parked; GH934 work remains capability-specific |
| D5 | Expiring integrate/continue/remove decision | Satisfied | G1 `5b98e80d` | manifest decision dates and overdue rejection | 2026-11-30 where listed; owner must decide |

### E. Dependency direction and packaging

| ID | Original criterion | Verdict | Implementation / exact SHA | Contract and regression evidence | Lifecycle, rollback, dependency |
|---|---|---|---|---|---|
| E1 | Establish target direction and remove key reverse dependencies | Amended | G2 `36f3d38c` | Technical target direction; 42 visible accepted reverse edges | No reduction claimed; future cleanup may shrink baseline |
| E2 | Replace broad facade wildcard exports | Amended | G2 `36f3d38c` | Guard prevents new direction/cycle debt; no facade rewrite in close slice | Future scoped cleanup; no public-API churn authorized here |
| E3 | Reject new cycles/layer violations | Satisfied | G2 `36f3d38c` | scanner/self-tests; fresh largest component = 37 | Baseline is shrink-only and cannot silently grow |
| E4 | Measure packaging economics before crate split | Satisfied as non-action | C0 `886e6eda`; G2 `36f3d38c` | Crate split remains a non-goal; no split decision was taken | Measurements required before any future split |
| E5 | Evaluate eval/local-onnx isolation before crate split | Satisfied as lifecycle decision | G1 `5b98e80d`; G2 `36f3d38c` | manifest classifies eval and local-onnx; no premature split | Reassess on model/default or packaging proposal |

### F. Eval decision model and product evidence

| ID | Original criterion | Verdict | Implementation / exact SHA | Contract and regression evidence | Lifecycle, rollback, dependency |
|---|---|---|---|---|---|
| F1 | Unified ship matrix explains all gate classes | Satisfied | G3 `257fc4a0` | executable merge/release/default/public-claim rows | Fails closed on missing or mismatched evidence |
| F2 | Require baseline/enhanced ablation before advanced defaults | Satisfied as gate, research independent | G1 `5b98e80d`; G3 `257fc4a0` | default-on row requires same-head ablation; incomplete Router stays experimental | GH934 or future capability owner supplies evidence |
| F3 | Preserve bounded graph evidence | Satisfied | G1 `5b98e80d`; G3 `257fc4a0` | graph inventory/report retained and authority-bound | Production graph remains bounded; rollback per inventory |
| F4 | Add user-outcome metrics | Satisfied | G3 `257fc4a0` | outcome scorecard fields for help, harm, repetition, injection, completion | Unavailable evidence is explicit, never fabricated |
| F5 | Enforce stop-loss on advanced capabilities | Satisfied as gate | G1 `5b98e80d`; G3 `257fc4a0` | stop-loss/default rows and manifest rollback/date fields | Future two-round evidence drives gate/rollback/removal |

### G. Documentation and contributor orientation

| ID | Original criterion | Verdict | Implementation / exact SHA | Contract and regression evidence | Lifecycle, rollback, dependency |
|---|---|---|---|---|---|
| G1 | Remove/generated volatile architecture tables | Satisfied | C0 `886e6eda` | architecture contains ownership map, not hand-maintained LOC counts | Current documentation contract |
| G2 | Document production flow, hosts, experiments, recovery | Satisfied | G6 `b800af4f` | Architecture “Runtime And Surface Lifecycle” | Links canonical manifest; no independent state |
| G3 | Canonicalize `docs/specs` vs root packets | Satisfied | G6 `b800af4f`, `5a9a98d1` | index affirmatively links every overlap as historical and rejects missing packets; 69 checker tests | `docs/specs/` is canonical; root packets retained as evidence |
| G4 | Check declarations, not only file presence | Satisfied | G1 `5b98e80d`; G4 `286326e0`; G6 `b800af4f` | surface lifecycle plus documentation contract checks | CI/preflight enforced |

### Existing issues included by reference

| ID | Original reference | Verdict | Implementation / exact SHA | Contract and regression evidence | Lifecycle, rollback, dependency |
|---|---|---|---|---|---|
| R1 | #932 Context Bundle | Repository-local v1 satisfied | G1 `5b98e80d`; G3 `257fc4a0` | GH932 contract, default bundle tests/audits | MCP API remains experimental; v2 dependency is #933 |
| R2 | #933 CurrentTruth | Independent parked | S3 `86fee409`; G1 `5b98e80d` | safe v1 production precursor is evidenced; breaking v2 contract remains open | Parked; does not block #969 repository reconciliation |
| R3 | #934 Retrieval Router | Partial, explicitly experimental | G1 `5b98e80d`; G3 `257fc4a0` | plan/routed-parameter tests; default-on gate remains closed | Capability-specific completion remains outside #969 |
| R4 | #953 SessionStart/retrieval convergence | Landed scoped slice | G1 `5b98e80d` | specs index records S1 shipped and later stages future | No extra claim manufactured for #969 |
| R5 | #684 legacy observation retirement | Re-audited and satisfied | S2 `99777754` | transactional idempotent compatibility projection | Recovery/deprecated lifecycle retained |
| R6 | #672 poisoning defense | Gap closed | S0 `044b0d04`; S1 `6836327d` | Dream boundary and production-path eval | Fail-closed quarantine |
| R7 | #632 adversarial policy | Gap closed | S1 `6836327d`; G5 `36f06d7a` | real DB pipeline and exact-main native artifacts | Public-claim authority remains G3 |
| R8 | #290 lifecycle planner | Bypass closed | G0 `80acc689` | sole activation API and bypass guard | Production continuous |

### Suggested implementation child issues

| ID | Original child | Verdict | Implementation / exact SHA | Contract and regression evidence | Lifecycle, rollback, dependency |
|---|---|---|---|---|---|
| I1 | Dream trust/quarantine | Satisfied | S0 `044b0d04` | Dream poisoning/provenance/review tests | Production |
| I2 | Project-aware candidate deduplication | Satisfied | S0 `044b0d04` | external identity and migration tests | Production |
| I3 | Production-path adversarial harness | Satisfied | S1 `6836327d` | persisted-state pipeline | Production eval |
| I4 | Legacy events retirement/idempotency | Satisfied | S2 `99777754` | projection tests | Deprecated compatibility |
| I5 | CurrentTruth relation integrity | Satisfied | S3 `86fee409` | cross-subject isolation | v1 production; #933 independent |
| I6 | Surface inventory and gates | Satisfied | G1 `5b98e80d` | manifest/guard | Continuous/date-bound |
| I7 | Dependency-direction guard | Satisfied | G2 `36f3d38c` | current and synthetic checks | No-expansion; shrink-only baseline |
| I8 | Ship matrix and scorecard | Satisfied | G3 `257fc4a0`; G5 `36f06d7a` | runtime authority and native evidence | Exact-head fail-closed |
| I9 | Architecture/spec synchronization | Satisfied | G6 `b800af4f` | architecture/index/checker tests | Current documentation contract |

### Done When

| ID | Original criterion | Verdict | Implementation / exact SHA | Contract and regression evidence | Lifecycle, rollback, dependency |
|---|---|---|---|---|---|
| Z1 | Stabilization contract accepted/indexed | Satisfied | C0 `886e6eda`; G6 `b800af4f` | current Product/Technical pair and index | Current |
| Z2 | Poisoned Dream output stays inactive/invisible | Satisfied | S0 `044b0d04` | poisoning, supersede, MCP/SessionStart exposure tests | Quarantine fail-closed |
| Z3 | Same topic imports remain repository-isolated | Satisfied | S0 `044b0d04` | external identity tests | Production |
| Z4 | Security E2E uses production/persisted state | Satisfied | S1 `6836327d`; G3 `257fc4a0` | production pipeline, snapshots, verifier | Exact evidence identity required |
| Z5 | Capture retry has unambiguous compatibility result | Satisfied | S2 `99777754` | atomic/idempotent projection tests | Deprecated compatibility retained |
| Z6 | Every major surface has complete lifecycle metadata | Satisfied | G1 `5b98e80d` | manifest and consistency guard | Owner/date/rollback per row |
| Z7 | No partial surface silently appears production | Satisfied | G1 `5b98e80d`; G3 `257fc4a0` | caller/default classification and ship matrix | Fail closed |
| Z8 | Direction guard active and top cycle reduced | Amended and measured | G2 `36f3d38c`; audit base `36f06d7a` | active guard; baseline = current = 37 | No reduction claim; future work may only shrink |
| Z9 | Advanced defaults have ablation/stop-loss evidence | Satisfied as admission gate | G1 `5b98e80d`; G3 `257fc4a0` | existing production evidence retained; incomplete surfaces stay non-default | #933/#935 parked; GH934 capability-specific |
| Z10 | Architecture/spec truth and no protected regression | Satisfied | G5 `36f06d7a`; G6 `b800af4f` | exact-main CI/native evidence plus docs/surface/dependency checks | Release/default/public claims remain separate gates |

## Fresh Close-Audit Verification

The #1050 implementation is accepted only after these commands pass on its
exact head:

```bash
python3 scripts/ci/check_public_surface.py
python3 scripts/ci/check_documentation_contracts.py
python3 scripts/ci/test_check_documentation_contracts.py
python3 scripts/ci/check_module_dependencies.py --base origin/main
python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md
```

Post-merge, #969 may close only after its GitHub body is reconciled to the
merged PR and exact merge SHA. Closing #969 must not close or unpark #931,
#933, or #935.
