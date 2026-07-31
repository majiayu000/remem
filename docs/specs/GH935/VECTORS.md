# Cross-Host Continuity Benchmark — Normative Vectors

Status: Current contract; executable implementation pending; Issue: #935

This document owns the fixed algorithms and independent vectors referenced by
`PRODUCT.md` and `TECH.md`. It does not claim that the current
`infrastructure_only_no_runs` implementation executes them. Implementation must
materialize the generator and the independently authored verifier under
`eval/cross-host/`; copied generator validation code is not independent.

## Common Framing

`JCS(x)` is the exact RFC 8785 encoding of I-JSON value `x`.

```text
u64be(n) = unsigned 64-bit big-endian n
frame(x) = u64be(len(x)) || x
H(tag, x...) = SHA-256(UTF8(tag) || 0x00 || frame(x[0]) || ... || frame(x[n]))
raw32(h) = the 32 bytes represented by lowercase hex64 h
```

Every invalid hex length, non-lowercase serialized digest, integer overflow,
duplicate key, invalid Unicode scalar value, non-finite number, or non-I-JSON
input is rejected before hashing.

## Release Fingerprint

The five committed inputs are:

```text
hidden_input_root
scoring_ir_hash
oracle_rules_hash
sanitizer_hash
deriver_hash
```

Each is a `hex64` digest of the exact claim-affecting bytes named by the sealed
plan. The fingerprint is:

```text
release_fingerprint_v1 =
  hex(H("cross-host-v2/release-fingerprint-v1",
        raw32(hidden_input_root),
        raw32(scoring_ir_hash),
        raw32(oracle_rules_hash),
        raw32(sanitizer_hash),
        raw32(deriver_hash)))
```

The generator must compute this value after all five digests exist. It may not
accept a fingerprint parameter or substitute a random value, revision name,
execution root, candidate ID, or registry key. The independent verifier
recomputes it from the five authenticated inputs and compares every downstream
copy before checking registry non-membership.

Fixed input digests:

| Input bytes | SHA-256 |
|---|---|
| `hidden-input-v1` | `dba0a3e812733ff7ea5a69473b1cfc7f1078dcf6a86eb44670ebb9866f836af6` |
| `scoring-ir-v1` | `cf83dbedbebe8bbb153af5aba0def69bc0e9f0a19dc035afeb047e570ad19151` |
| `oracle-rules-v1` | `9c60404724cef8960f9405c1d7c692c6a73972ba33ff84669026332f145da5ea` |
| `sanitizer-v1` | `809b8a9270dbcc6f1e4a161d29dffaee100c98d898d4f94829352299bc1abb58` |
| `deriver-v1` | `df32f999d824be1ea0b21dea1ab0a218e355baa4ef45bc5d4994ba14a910d9f1` |

The resulting fingerprint is
`1e13d405471fbd0f40f5379c71283681102ecb7572e32a9ccdf94c35f3211ccf`.
Changing any one input must change it; keeping it unchanged is a verifier
failure, not another acceptable fixture.

## Independent Schema-Mutation Execution

The generator emits only the positive object set and the authenticated mutation
manifest. It must not emit a list saying that mutations were rejected.

The verifier owns a separately implemented closed-schema validator. For each
manifest row it must:

1. clone the authenticated positive object set;
2. apply the specified operation at the exact JSON Pointer;
3. invoke the independent validator on the complete mutated set;
4. require rejection with the registered reason code; and
5. prove the unmodified positive set still validates.

Comparing case names, booleans, or rejection output produced by the generator is
forbidden. A mutation that cannot be applied, is rejected only by a frozen file
digest, or reaches the wrong reason code fails the verifier.

The closed manifest is:

| Case | Object / JSON Pointer | Mutation | Required rejection |
|---|---|---|---|
| `condition_number` | `runs/run_v2.json` `/condition` | replace with number `7` | `schema/type` |
| `source_unknown_field` | `attempts/source_attempt_v2.json` `/unknown_source_field` | add string | `schema/additional_property` |
| `illegal_attempt_selection` | `runs/run_v2.json` `/attempt_evidence/selected_claim_attempt` | replace with `other-attempt` | `binding/attempt` |
| `manifest_missing_count` | `evidence/evidence_manifest_v2.json` `/missing_tuple_count` | replace with `432` | `binding/count_partition` |
| `task_category_enum` | `inputs/task_v2.json` `/category` | replace with `definitely_not_a_category` | `schema/enum` |
| `oracle_result_hash_drift` | `scoring/public_scoring_projection_v1.json` `/oracle_result_hash` | replace with zero hex64 | `binding/oracle_hash` |
| `proof_extra_sibling` | same object `/reads/0/sibling_path/-` | append zero hex64 | `binding/nonminimal_proof` |
| `general_history_unknown_field` | `publication/registry_general_history_vector_v1.json` `/unknown_history_field` | add string | `schema/additional_property` |
| `freeze_unknown_field` | `publication/final_envelope_freeze_v1.json` `/unknown_freeze_field` | add string | `schema/additional_property` |
| `visibility_receipt_unknown` | `publication/visibility_proof_suite_v1.json` `/receipt/unknown_nested_field` | add string | `schema/additional_property` |
| `freeze_ledger_replay_unknown` | `publication/final_envelope_freeze_ledger_vectors_v1.json` `/replay_result/unknown_nested_field` | add string | `schema/additional_property` |
| `freeze_ledger_empty_drift` | same object `/drift_rejections` | replace with `[]` | `schema/min_items` |
| `envelope_core_member_unknown` | `publication/final_publication_envelope_v1.json` `/core_members/0/unknown_nested_field` | add string | `schema/additional_property` |
| `registry_cas_receipt_unknown` | same object `/registry_proof/cas_receipt/unknown_nested_field` | add string | `schema/additional_property` |
| `registry_cas_request_hash_drift` | same object `/registry_proof/cas_request_hash` | replace with zero hex64 | `binding/cas_request_hash` |
| `registry_cas_request_unknown` | same object `/registry_proof/cas_request/unknown_nested_field` | add string | `schema/additional_property` |
| `registry_transition_leaf_unknown` | `publication/registry_general_history_vector_v1.json` `/transition_leaves/0/unknown_nested_field` | add string | `schema/additional_property` |

The verifier also mutates `release_fingerprint`, every read receipt field and
signature, every RFC 8785 expected byte, and every private-byte registry limit.
Those checks are separate from the 17 compatibility cases above.

## RFC 8785 and I-JSON Vectors

Expected bytes are external oracles, not output copied from the generator.
The independent verifier parses each input with duplicate-key and invalid
Unicode detection, canonicalizes it with an implementation independent from
the generator, and byte-compares the result.

| Input JSON | Expected UTF-8 canonical bytes |
|---|---|
| `{"b":1,"a":2}` | `{"a":2,"b":1}` |
| `[333333333.33333329,1E30,4.50,2e-3,0.000000000000000000000000001]` | `[333333333.3333333,1e+30,4.5,0.002,1e-27]` |
| `[-0,0.0,1e-7,0.000001]` | `[0,0,1e-7,0.000001]` |
| `{"\u20ac":"euro","\r":"cr","\ufb33":"hebrew","1":"one","\ud83d\ude00":"emoji","\u0080":"control","\u00f6":"latin"}` | `{"\r":"cr","1":"one","\u0080":"control","ö":"latin","€":"euro","😀":"emoji","דּ":"hebrew"}` |
| `{"s":"quote:\\\" slash:/ backslash:\\\\ control:\\b\\f\\n\\r\\t"}` | `{"s":"quote:\\\" slash:/ backslash:\\\\ control:\\b\\f\\n\\r\\t"}` |

Required rejection vectors are duplicate keys (`{"a":1,"a":2}`), `NaN`,
`Infinity`, `1e400`, a lone `\ud800`, malformed UTF-8, and an integer whose
exact value is not representable by the configured IEEE-754 binary64 parser.
Python `json.dumps(sort_keys=True)` is not an RFC 8785 implementation and may
not be used as the verifier's canonicalizer.

## Private-Byte Encoding Registry

`private_byte_encoding_registry_v1` begins with the exact private byte string
and recursively applies this closed transform set:

1. raw bytes;
2. lowercase and uppercase hexadecimal;
3. RFC 4648 standard and URL-safe base64, each padded and unpadded;
4. byte-wise percent encoding using uppercase hex, plus percent-decoding of an
   artifact before the next scan;
5. JSON string unescaping followed by strict UTF-8 encoding;
6. strict UTF-8 text under NFC, NFD, NFKC, and NFKD;
7. gzip/RFC 1952, zlib/RFC 1950, ZIP store/deflate, and POSIX ustar archives.

No other archive format is claim-bearing in v1. ZIP and tar entry paths must be
relative POSIX paths; duplicate paths, links, devices, absolute paths, `..`,
encrypted entries, concatenated gzip members, trailing undecoded bytes, CRC
failure, or malformed headers are unclassifiable.

The limits are part of the verdict:

| Limit | Value |
|---|---:|
| Recursive transform depth | 4 |
| Archive entries per object | 128 |
| Decoded bytes per entry | 16 MiB |
| Aggregate decoded bytes per artifact | 64 MiB |
| Expansion ratio | 100:1 |
| Distinct derived candidates | 1024 |

Reaching any limit yields `partial_non_security` / `INSUFFICIENT`; it never
proves absence. Plans may add a separately reported diagnostic transform, but
claim-bearing v1 cannot remove, rename, or weaken a registered transform.

For private bytes `72 65 6d 65 6d 2f 70 72 69 76 61 74 65 00`:

| Transform | Fixed result |
|---|---|
| SHA-256 of raw bytes | `7d9698108dda9241224e7490659049637ec541110c9b3a1c99fdf39b4c5a4265` |
| Lower hex | `72656d656d2f7072697661746500` |
| Upper hex | `72656D656D2F7072697661746500` |
| Standard/URL-safe base64 padded | `cmVtZW0vcHJpdmF0ZQA=` |
| Standard/URL-safe base64 unpadded | `cmVtZW0vcHJpdmF0ZQA` |
| Percent encoding | `remem%2Fprivate%00` |
| JSON escaped text | `"remem/private\u0000"` |
| Deterministic gzip (DEFLATE level 6, `MTIME=0`, `XFL=0`, `OS=255`) | `1f8b08000000000000ff2b4acd4dcdd52f28ca2c4b2c496500002b266f610e000000` |
| zlib level-6 bytes | `789c2b4acd4dcdd52f28ca2c4b2c4965000029d30541` |

The scanner must find the raw bytes after decoding every positive vector.
One-over-limit variants must yield `INSUFFICIENT`; a one-byte private-value
mutation must not match.

## Observed Read Receipts

Visibility metadata available before a read is not read evidence. Every
successful `read_visible` call returns:

```text
read_result_v1 = {
  "byte_length": dec,
  "path": canonical_path,
  "sha256": hex64
}
```

The authority sorts results by UTF-8 path bytes and signs the closed receipt:

```text
read_verification_receipt_v1 = {
  "core_evidence_root": hex64,
  "fingerprint": hex64,
  "object_set_root": hex64,
  "reads": [read_result_v1, ...],
  "version": "read_verification_receipt_v1",
  "visibility_checkpoint_hash": hex64
}

signature_input =
  ASCII("cross-host-v2/read-verification-receipt-v1") || 0x00 ||
  u64be(len(JCS(receipt))) || JCS(receipt)

read_verification_root_v1 =
  hex(H("cross-host-v2/read-verification-root-v1",
        raw32(object_set_root),
        raw32(final_publication_envelope_hash),
        SHA-256(JCS(receipt)),
        raw64(receipt_signature)))
```

The receipt must contain exactly the final envelope and every core member, with
no duplicate, missing, extra, aliased, or reordered path. The independent
verifier reads each published byte through the gated API, recomputes each
length/digest, verifies the authority signature and checkpoint, then recomputes
the root. A runner-created list or a root over expected metadata is invalid.

The fixed Ed25519 public-key SPKI for the read vector is
`302a300506032b6570032100ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c`.
Its two sorted results are:

| Path | Length | SHA-256 |
|---|---:|---|
| `core/a.json` | 13 | `f12ca340de001b036a14b981e8e9afa8ab12dd3d3464016e7c5550386b309586` |
| `final_publication_envelope_v1.json` | 21 | `2bdc2c84169b6cbc8eec068af61dcaf655a41fdfe5e0e72ebdec590623d7328e` |

For fingerprint
`1e13d405471fbd0f40f5379c71283681102ecb7572e32a9ccdf94c35f3211ccf`,
the 661-byte receipt SHA-256 is
`d8d1f9ac224a262bdfe105006116d5b471425a2006e9c3df7cd3c840c3db1c48`;
its signature is
`df7c1f1f2de10d8faec649de4f5d5a6c80b95c09e243c43e492fc87625d3e036835a44485d75364cb0995b38b41caafb6e2cf0dd91a7ede9950c1632fe6f4708`;
the final read root is
`cc006ed1f99cf546d472ea75ea1d2d06f3baf758345592021d92f5ce9559a10c`.

Mutating either path, length, digest, checkpoint, receipt signature, or one
published byte must fail. Omitting the gated reads while retaining expected
metadata must also fail.

## Implementation Verification

The future implementation PR must run all of the following offline:

```text
generate positive production-shaped object set
independently validate the positive set
independently apply and reject every schema mutation
recompute the release fingerprint from authenticated inputs
run every RFC 8785 and invalid-I-JSON vector
decode every private-byte positive and limit vector
perform gated reads and verify the signed receipt/root
```

The generator and verifier must be separate source files with no shared
validator, canonicalizer, transform registry, or expected-value module. CI
must execute both from a clean checkout. A list of claimed passes, a generator
boolean, frozen output digest, or review comment is not verification evidence.
