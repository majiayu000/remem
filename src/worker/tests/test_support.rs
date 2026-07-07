use std::os::unix::fs::PermissionsExt;

pub(super) fn install_stub_codex(path: &std::path::Path) {
    let script = r#"#!/bin/sh
prev=""
output_path=""
for arg in "$@"; do
  if [ "$prev" = "--output-last-message" ]; then
    output_path="$arg"
    break
  fi
  prev="$arg"
done
if [ -z "$output_path" ]; then
  echo "missing output path" >&2
  exit 1
fi
stdin_path="${TMPDIR:-/tmp}/remem-stub-codex-$$.txt"
cat > "$stdin_path"
if grep -q "Task: memory_candidate" "$stdin_path"; then
cat <<'EOF' > "$output_path"
<memory_candidate>
  <scope>project</scope>
  <type>decision</type>
  <topic_key>codex-worker-flush</topic_key>
  <risk_class>low</risk_class>
  <confidence>0.91</confidence>
  <text>Queued Codex observation persisted.</text>
</memory_candidate>
EOF
rm -f "$stdin_path"
exit 0
fi
if grep -q "Task: graph_candidate" "$stdin_path"; then
cat <<'EOF' > "$output_path"
<no_graph_candidates reason="stub has no graph facts"/>
EOF
rm -f "$stdin_path"
exit 0
fi
if grep -q "REPLACE_WITH_TOPIC_KEY" "$stdin_path"; then
cat <<'EOF' > "$output_path"
<summary>Codex worker flush completed.</summary>
<structured_fields>
  <request>Codex worker flush</request>
  <decisions>Queued Codex observation persisted.</decisions>
  <learned></learned>
  <next_steps></next_steps>
  <preferences></preferences>
</structured_fields>
<segments></segments>
EOF
rm -f "$stdin_path"
exit 0
fi
cat <<'EOF' > "$output_path"
{
  "observations": [
    {
      "type": "decision",
      "title": "Codex worker flush",
      "subtitle": null,
      "narrative": "Queued Codex observation persisted.",
      "facts": [],
      "concepts": [],
      "files_read": [],
      "files_modified": [],
      "confidence": 0.9
    }
  ]
}
EOF
rm -f "$stdin_path"
"#;
    std::fs::write(path, script).expect("stub codex script should be written");
    let mut perms = std::fs::metadata(path)
        .expect("stub codex metadata should load")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("stub codex permissions should be set");
}
