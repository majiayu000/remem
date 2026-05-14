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
Type "export PATH=$PWD/target/release:$PATH"
Enter
Type "export HOME=/private/tmp/remem-demo-home"
Enter
Type "export REMEM_DATA_DIR=/tmp/remem-vhs-demo"
Enter
Type "export REMEM_STDERR_TO_LOG=1"
Enter
Type "export REMEM_CONTEXT_HOST=codex-cli"
Enter
Type "export DEMO_PROJECT=/private/tmp/remem-codex-project"
Enter
Type "rm -rf $REMEM_DATA_DIR $HOME $DEMO_PROJECT && mkdir -p $REMEM_DATA_DIR $HOME/.codex $DEMO_PROJECT"
Enter
Type "remem preferences add --project $DEMO_PROJECT 'Prefer pnpm for JavaScript package management' >/dev/null"
Enter
Type "remem preferences add --project $DEMO_PROJECT 'Run cargo test before release builds' >/dev/null"
Enter
Type "clear"
Enter
Show

Type "printf 'Demo uses temp HOME + temp database; no private memories are shown.\\n'"
Enter
Sleep 2s

Type "remem install --target codex"
Enter
Sleep 4s

Type "sed -n '1,8p' ~/.codex/config.toml"
Enter
Sleep 4s

Type "echo"
Enter
Type "echo 'What Codex receives on SessionStart:'"
Enter
Sleep 1s

Type "remem context --cwd $DEMO_PROJECT --host codex | head -18"
Enter
Sleep 9s
