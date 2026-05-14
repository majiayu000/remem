use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: Option<&str>) -> Self {
        let previous = std::env::var_os(key);
        match value {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        match self.previous.as_ref() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

pub(super) struct ScopedEnv {
    _guard: MutexGuard<'static, ()>,
    _vars: Vec<ScopedEnvVar>,
}

impl ScopedEnv {
    pub(super) fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
        let guard = ENV_LOCK.lock().expect("env lock should acquire");
        let vars = vars
            .iter()
            .map(|(key, value)| ScopedEnvVar::set(key, *value))
            .collect();
        Self {
            _guard: guard,
            _vars: vars,
        }
    }
}

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
cat <<'EOF' > "$output_path"
<observation>
  <type>decision</type>
  <title>Codex worker flush</title>
  <narrative>Queued Codex observation persisted.</narrative>
</observation>
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
