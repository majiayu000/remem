#!/usr/bin/env bash
# Seeds the demo repo + temp remem DB used by assets/demo-recall.tape.
# Uses the installed `remem` on PATH; data goes to /tmp/remem-vhs-demo3 only.
set -euo pipefail

DEMO_PROJECT=/private/tmp/remem-demo-webapp
export REMEM_DATA_DIR=/tmp/remem-vhs-demo3
export REMEM_ALLOW_PLAINTEXT_DB=1 REMEM_STDERR_TO_LOG=1
DEMO_BACKUP=$REMEM_DATA_DIR/seed-backup

rm -rf "$REMEM_DATA_DIR" "$DEMO_PROJECT"
mkdir -p "$REMEM_DATA_DIR" "$DEMO_PROJECT/src"

cd "$DEMO_PROJECT"
cat > package.json <<'EOF'
{ "name": "shop-webapp", "version": "1.4.2", "scripts": { "test:e2e": "playwright test" } }
EOF
cat > src/session.ts <<'EOF'
export const SESSION_SCHEMA = "v2";

export async function readSession(key: string, opts: { fallback?: "v1" } = {}) {
  // v2 first; fall back to v1 keys written before the rollback window
  return null as any; // demo stub
}
EOF
cat > src/checkout.ts <<'EOF'
import { readSession } from "./session";

export async function checkout(cartId: string, sessionKey: string) {
  // SESSION_SCHEMA v2 everywhere; fallback reader handles legacy v1 keys
  const session = await readSession(sessionKey, { fallback: "v1" });
  if (!session) throw new Error("checkout: no session");
  return { ok: true, cartId, userId: session.userId };
}
EOF
git init -q
git add -A
git -c user.email=dev@example.com -c user.name=dev commit -qm "fix(checkout): bump SESSION_SCHEMA to v2 + v1 fallback reader"
COMMIT=$(git rev-parse --short HEAD)

NOW=$(date +%s); FRI=$((NOW-259200))
sqlite3 "$DEMO_BACKUP" <<SQL
CREATE TABLE memories (id INTEGER PRIMARY KEY, project TEXT NOT NULL, topic_key TEXT, title TEXT NOT NULL, content TEXT NOT NULL, memory_type TEXT NOT NULL, created_at_epoch INTEGER NOT NULL, updated_at_epoch INTEGER NOT NULL, status TEXT, branch TEXT, scope TEXT);
INSERT INTO memories (project, topic_key, title, content, memory_type, created_at_epoch, updated_at_epoch, status, branch, scope) VALUES
('$DEMO_PROJECT', 'checkout-500-root-cause', 'Checkout 500s: stale Redis session schema', 'Root cause: checkout read v2 session keys while auth still wrote v1 after a rollback. Fix: bump SESSION_SCHEMA to v2 + fallback reader (commit $COMMIT). TODO: regression test for the v1 fallback path.', 'bugfix', $FRI, $FRI, 'active', 'main', 'project'),
('$DEMO_PROJECT', 'payments-idempotency-decision', 'Use idempotency keys on /api/pay', 'Decided: every POST /api/pay carries an idempotency key derived from cart hash; retries must never double-charge.', 'decision', $FRI, $FRI, 'active', 'main', 'project');
SQL
remem import backup --source "$DEMO_BACKUP" --best-effort >/dev/null
sqlite3 "$REMEM_DATA_DIR/remem.db" "INSERT INTO workstreams (project, title, description, status, progress, next_action, created_at_epoch, updated_at_epoch) VALUES ('$DEMO_PROJECT', 'Checkout hardening', 'Stabilize checkout after the session-schema incident', 'active', 'Fixed v1/v2 session mismatch; fallback reader shipped', 'Add regression test for the v1 fallback path', $FRI, $FRI);"
remem preferences add --project "$DEMO_PROJECT" 'Run pnpm test:e2e before any checkout change' >/dev/null
echo "seeded: $REMEM_DATA_DIR + $DEMO_PROJECT (commit $COMMIT)"
