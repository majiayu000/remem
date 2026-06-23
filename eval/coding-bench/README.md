# Coding-Agent A/B Benchmark Artifact Contract

This directory is reserved for the #385 coding-agent A/B benchmark runner and
reports. The runner is not implemented here yet; the current implementation
defines the artifact schema that #385 must use for current-memory evidence.

Every `remem` run artifact must include `remem_contract_snapshot`, built from
the current-memory-contracts deterministic report. That snapshot records
contract health, citation precision, usage feedback coverage, injection audit
coverage, temporal fact eligibility, and staleness/source-anchor handling.

`no_memory` and `curated_file` runs must set `memory_contract_status` to
`not_applicable` and must not include remem contract evidence.

Runtime contract failure is separate from agent task failure. A run may solve
the coding task while still failing the remem runtime contract; reports must
preserve both facts instead of merging them into one failure reason.
