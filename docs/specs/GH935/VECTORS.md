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
4. RFC 3986 unreserved bytes left literal and every other byte percent-encoded
   with uppercase hex, plus percent-decoding of an artifact before the next scan;
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
| Standard base64 padded/unpadded | `cmVtZW0vcHJpdmF0ZQA=` / `cmVtZW0vcHJpdmF0ZQA` |
| URL-safe base64 padded/unpadded | `cmVtZW0vcHJpdmF0ZQA=` / `cmVtZW0vcHJpdmF0ZQA` |
| Percent encoding | `remem%2Fprivate%00` |
| JSON escaped text | `"remem/private\u0000"` |
| Deterministic gzip (DEFLATE level 6, `MTIME=0`, `XFL=0`, `OS=255`) | `1f8b08000000000000ff2b4acd4dcdd52f28ca2c4b2c496500002b266f610e000000` |
| zlib level-6 bytes | `789c2b4acd4dcdd52f28ca2c4b2c4965000029d30541` |

Alphabet-disambiguation bytes `fb ff` encode as standard `+/8=` / `+/8` and
URL-safe `-_8=` / `-_8`; accepting one alphabet must not silently relabel the
other.

The normalization input code points are
`U+2460 U+0020 U+212B U+0020 U+FB01 U+0020 U+0065 U+0301`. Exact results are:

| Form | Result code points | UTF-8 hex |
|---|---|---|
| NFC | `U+2460 U+0020 U+00C5 U+0020 U+FB01 U+0020 U+00E9` | `e291a020c38520efac8120c3a9` |
| NFD | `U+2460 U+0020 U+0041 U+030A U+0020 U+FB01 U+0020 U+0065 U+0301` | `e291a02041cc8a20efac812065cc81` |
| NFKC | `U+0031 U+0020 U+00C5 U+0020 U+0066 U+0069 U+0020 U+00E9` | `3120c38520666920c3a9` |
| NFKD | `U+0031 U+0020 U+0041 U+030A U+0020 U+0066 U+0069 U+0020 U+0065 U+0301` | `312041cc8a2066692065cc81` |

Archive positives contain one regular entry `private.bin`, mode `0600`,
uid/gid/mtime `0`, empty owner/group names, no extra/comment, and the 14 private
bytes. ZIP uses DOS time `1980-01-01T00:00:00`, Unix creator `3`, no Zip64/data
descriptor; deflate is level 6. Ustar is the minimal header, one padded data
block, and two zero blocks. Exact archive bytes are:

| Format | Length / SHA-256 | Exact byte construction |
|---|---|---|
| ZIP store | `134` / `68c0266d2cefcc2d7b2feb2fffd467eb8c8c0a533f0e8fa6a937f5a53b6c3840` | base64-decode `UEsDBBQAAAAAAAAAIQArJm9hDgAAAA4AAAALAAAAcHJpdmF0ZS5iaW5yZW1lbS9wcml2YXRlAFBLAQIUAxQAAAAAAAAAIQArJm9hDgAAAA4AAAALAAAAAAAAAAAAAACAgQAAAABwcml2YXRlLmJpblBLBQYAAAAAAQABADkAAAA3AAAAAAA=` |
| ZIP deflate | `136` / `4675da0bff54f428d7ff79632e530d1bd90d09b5b5dce3d850a289412139ef3c` | base64-decode `UEsDBBQAAAAIAAAAIQArJm9hEAAAAA4AAAALAAAAcHJpdmF0ZS5iaW4rSs1NzdUvKMosSyxJZQAAUEsBAhQDFAAAAAgAAAAhACsmb2EQAAAADgAAAAsAAAAAAAAAAAAAAICBAAAAAHByaXZhdGUuYmluUEsFBgAAAAABAAEAOQAAADkAAAAAAA==` |
| POSIX ustar | `2048` / `7dd3b603ee643f07687ce689b3353e65428d5dafb1e0dff151149d9bad4c0f74` | base64-decode header `cHJpdmF0ZS5iaW4AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADAwMDA2MDAAMDAwMDAwMAAwMDAwMDAwADAwMDAwMDAwMDE2ADAwMDAwMDAwMDAwADAxMDA3NgAgMAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB1c3RhcgAwMAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=`, then append the 14 private bytes, 498 zero bytes, and two 512-byte zero blocks |

Limit tests use the same encoders and exact recipes below. ZIP-store bodies
isolate byte/count limits from expansion; all names and list order are literal.

| One-over-limit fixture | Exact recipe | Length / SHA-256 |
|---|---|---|
| Depth 5 | Apply zlib level 6 five times to the 14 private bytes; final hex `789cab98c3a8c970ed7fc51c46398687406af58cc56f4baf7d7f135adb1b6c75cf6ade4a1e9e59d3236519330fb2ff624fe7f7d9c92bac01009db7173c` | `61` / `534adf0753e165fa974f63b39a43c52c4302c3b5dfb0b5f17d0f844e73cded20` |
| 129 entries | ZIP store empty entries `entry/000` through `entry/128` | `12148` / `8c3c5fad33a6caf6ede2f07bc6949c7e7ceb5ff237712be07aeea0dc9f9b1b34` |
| 16 MiB + 1 entry | ZIP store `large.bin = ASCII("A") * 16777217` | `16777333` / `17e5c6f79bcbe70335cfab0cc092765613c886fe720a9f6d68497165f9066729` |
| 64 MiB + 1 aggregate | ZIP store `part/0..3 = ASCII("A") * 16777216`, `part/4 = ASCII("Z")` | `67109327` / `2f26275e71d6af1bc2352e0e0ae16ee8b05de6e3add2feee827c05855eb7b87c` |
| Expansion >100:1 | ZIP deflate `ratio.bin = ASCII("A") * 1048576` | `1150` / `079f7e8feeb893a1747042a06b5d4e078544b26e97c889f756f9cfacd3d36ddf` |
| 1025 candidates | Compact UTF-8 JSON array of `candidate-0000` through `candidate-1024`; each string token enters the transform worklist | `17426` / `0c97dee42739384c4c1191edf03e6a02608fa86672558c3199985f7683977fb5` |

The scanner must find the raw bytes after every positive vector and must return
`INSUFFICIENT` for each one-over-limit fixture before scanning beyond the
limit. A one-byte private-value mutation must not match.

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

The fixed vector uses these exact published/checkpoint bytes:

| Object | Exact UTF-8 hex | Length / SHA-256 |
|---|---|---|
| `core/a.json` | `636f72652d612d62797465730a` | `13` / `f12ca340de001b036a14b981e8e9afa8ab12dd3d3464016e7c5550386b309586` |
| `final_publication_envelope_v1.json` | `66696e616c2d656e76656c6f70652d62797465730a` | `21` / `2bdc2c84169b6cbc8eec068af61dcaf655a41fdfe5e0e72ebdec590623d7328e` |
| authenticated checkpoint fixture | `7669736962696c6974792d636865636b706f696e742d76310a` | `25` / `787ff79210c014c8c7dd37a7abc3ed0ee6a0be59d417150023bcd6d9f33772f5` |

The one-member core-leaf framing produces `core_evidence_root =
bf1150ae10aead450799c7357512ba521ba39956a128b843b9ddc6fa494757ad`.
The final-envelope framing produces
`9b6168bc83932006ee15f6379802b488e4b1cb35cc9bea374d16b7c3d5b3efd8`;
the visibility object-set formula produces
`7aa43ff551ddbd1b9bc04468012ae61436d4edeca20a0a43f0f2c9fb0b95bb80`.
With fingerprint `1e13d405471fbd0f40f5379c71283681102ecb7572e32a9ccdf94c35f3211ccf`,
the JCS receipt is exactly 657 bytes with SHA-256
`eba7791918ad3abf39f4a69692daaf0eefdd821ce1d711f9f16af96eba69dc32`.

The Ed25519 public-key SPKI is
`302a300506032b6570032100ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c`;
the receipt signature is
`9053d37099c5b926df5afba0e7c9cc16a9fe322877e36cc08f3f92a73a571d3ce0fd4222fc0e2db125518434faa97492fab584203f9a6de2929f3fca40578802`;
the final read root is
`321c6c70d353b3ae470b6ec078f93b97a7cf954d77c0049a2457e2359b85d72c`.

Mutating either path, length, digest, checkpoint, receipt signature, or one
published byte must fail. Omitting the gated reads while retaining expected
metadata must also fail.

## Completion Record

The completion identifier and record hash are independent domain-separated
values:

```text
completion_id =
  hex(H("cross-host-v2/publication-completion-id-v1",
        raw32(release_fingerprint),
        raw32(final_publication_envelope_hash),
        raw32(visibility_receipt_hash)))

completion_record_hash =
  hex(H("cross-host-v2/publication-complete-v1",
        JCS(publication_complete_v1)))
```

`previous_completion_hash` is the exact `completion_record_hash` of the
authenticated prior row. It is 64 zeroes only when the authenticated ledger
head proves that no prior row exists. An exact replay requires the same
`completion_id`, JCS bytes, and record hash; a duplicate ID with any drift or a
non-genesis zero previous hash is rejected.

The fixed vector uses the read-vector roots above. Each other nonzero fixture
digest below is `SHA-256(UTF8(label || "\n"))`, where `label` is the literal
left column:

| Label / field | Digest |
|---|---|
| `candidate-verdict-v1` / `candidate_verdict_hash` | `7525e74ae3a93ee783bf1b6a43f1072e3f859d1a0b3c65d7cf231186a4e1a023` |
| `final-envelope-freeze-v1` / `final_envelope_freeze_hash` | `d97e1aae094a4d7014e6b02459a6ac4e03891df4a3535298a9231704cc184360` |
| `registry-checkpoint-certificate-v1` / `registry_checkpoint_certificate_hash` | `41982fbfc0dcb441fe15ca491fb2514b89bc070280d48bce9c43217b4cbb1e0d` |
| `registry-post-root-v1` / `registry_post_root` | `61fc54ea4e0c55f97a9ef9aabeffdde454852225c122cd490b20816abd423a2b` |
| `visibility-checkpoint-certificate-v1` / `visibility_checkpoint_certificate_hash` | `ab1663b2e27cbd92d38475ecef8a8dd0ea640c60f64f60b27d7e9371e85b1f19` |
| `visibility-proof-suite-v1` / `visibility_proof_suite_hash` | `047aaea1853002a8fcadecde9e591a33795674118b0703063883a8ff0c4c83ea` |
| `visibility-receipt-v1` / `visibility_receipt_hash` | `e299de5a25f0e1c1ddfebaa769609c5c99ad20ac33dd66263d2c37872dc14f5f` |

For the authenticated-empty-ledger case, `previous_completion_hash` is 64
zeroes. With the fixed fingerprint, core root, final-envelope hash, object-set
root, and read root above, the derived `completion_id` is
`701dee4677c96de280fd29f3a8f37a92e260e7cec91c3a659cb7980a5dac53ab`.
The closed `publication_complete_v1` object in the field order declared by
`TECH.md` canonicalizes to 1349 JCS bytes and its `completion_record_hash` is
`7b0158d2c59c047326f886b58efe94c4bc915e82e6c1c20387057bb4a73b1300`.

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
recompute the completion ID and record hash, including genesis/replay rejection
```

The generator and verifier must be separate source files with no shared
validator, canonicalizer, transform registry, or expected-value module. CI
must execute both from a clean checkout. A list of claimed passes, a generator
boolean, frozen output digest, or review comment is not verification evidence.
