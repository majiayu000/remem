Output assets/remem-recall-demo.gif

Require bash

Set Shell "bash"
Set FontSize 19
Set Width 1200
Set Height 760
Set Theme "Dracula"
Set Padding 18
Set TypingSpeed 28ms
Set Framerate 24

# Prereqs (run once before `vhs assets/demo-recall.tape`):
#   scripts/demo/seed-recall-demo.sh   (seeds /tmp/remem-vhs-demo3 + demo repo)
# The claude call is a real Claude Code session; the API wait is Hidden.

Hide
Type "export REMEM_DATA_DIR=/tmp/remem-vhs-demo3 REMEM_ALLOW_PLAINTEXT_DB=1"
Enter
Type `claude() { command claude --dangerously-skip-permissions "$@"; }`
Enter
Type "cd /private/tmp/remem-demo-webapp"
Enter
Type "clear"
Enter
Show

Type "echo 'Live recording - real Claude Code session, demo repo, seeded remem memory.'"
Enter
Sleep 1.5s

Type "git log --oneline -1"
Enter
Sleep 1s
Type "echo 'Last week: you and Claude fixed this. Session ended. remem saved the context.'"
Enter
Sleep 3s

Type "echo 'Today: brand-new session. Ask Claude what you were doing:'"
Enter
Sleep 1.5s

Type `claude -p "I'm back on this project after a few days. What were we working on, and what's the next step? Answer in English, briefly."`
Enter
Sleep 2s
Hide
Wait@240s
Show
Sleep 12s

Type "echo 'Root cause, commit, open TODO - recalled, cited, zero re-explaining.'"
Enter
Sleep 4s
