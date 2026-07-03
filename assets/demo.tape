Output assets/remem-demo.gif

Require bash

Set Shell "bash"
Set FontSize 22
Set Width 1200
Set Height 720
Set Theme "Dracula"
Set Padding 18
Set TypingSpeed 35ms
Set Framerate 24

Hide
Type "export PATH=$PWD/target/release:/usr/bin:/bin"
Enter
Type "export HOME=/private/tmp/remem-demo-home"
Enter
Type "export REMEM_DATA_DIR=/tmp/remem-vhs-demo"
Enter
Type "export REMEM_ALLOW_PLAINTEXT_DB=1"
Enter
Type "export REMEM_STDERR_TO_LOG=1"
Enter
Type "export DEMO_PROJECT=/private/tmp/remem-codex-project"
Enter
Type "export DEMO_BACKUP=$REMEM_DATA_DIR/seed-backup"
Enter
Type "rm -rf $REMEM_DATA_DIR $HOME $DEMO_PROJECT && mkdir -p $REMEM_DATA_DIR $HOME/.codex $DEMO_PROJECT"
Enter
Type "NOW=$(date +%s)"
Enter
Type "sqlite3 $DEMO_BACKUP <<SQL"
Enter
Type "CREATE TABLE memories (id INTEGER PRIMARY KEY, project TEXT NOT NULL, topic_key TEXT, title TEXT NOT NULL, content TEXT NOT NULL, memory_type TEXT NOT NULL, created_at_epoch INTEGER NOT NULL, updated_at_epoch INTEGER NOT NULL, status TEXT, branch TEXT, scope TEXT);"
Enter
Type "INSERT INTO memories (project, topic_key, title, content, memory_type, created_at_epoch, updated_at_epoch, status, branch, scope) VALUES ('$DEMO_PROJECT', 'codex-hooks-context-injection', 'Codex hooks inject project context', 'remem installs Codex SessionStart and Stop hooks so new sessions receive relevant project memory automatically.', 'architecture', $NOW, $NOW, 'active', 'main', 'project'), ('$DEMO_PROJECT', 'release-test-gate', 'Run release tests before shipping', 'Before publishing a release, run cargo test and the plugin version sync checks.', 'decision', $NOW, $NOW, 'active', 'main', 'project');"
Enter
Type "SQL"
Enter
Type "remem import backup --source $DEMO_BACKUP --best-effort >/dev/null"
Enter
Type "remem preferences add --project $DEMO_PROJECT 'Prefer pnpm for JavaScript package management' >/dev/null"
Enter
Type "clear"
Enter
Show

Type "echo 'Demo uses a temp HOME + temp database; no private memories are shown.'"
Enter
Sleep 2s

Type "remem install --target codex"
Enter
Sleep 3s

Type "echo"
Enter
Type "echo 'New Codex SessionStart receives:'"
Enter
Sleep 1s

Type "remem context --cwd $DEMO_PROJECT --host codex-cli --force"
Enter
Sleep 8s
