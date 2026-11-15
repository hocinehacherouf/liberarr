# The README demo, on fictional data only. From the repository root:
#   cargo build -p liberarr && cargo build -p liberarr --example demo_home
#   vhs docs/assets/demo.tape
# Needs https://github.com/charmbracelet/vhs. The demo home must not exist yet.
Output docs/assets/demo.gif
Set Shell bash
Set FontSize 16
Set Width 1200
Set Height 720
Set Padding 14
Set Theme "Catppuccin Mocha"
Set TypingSpeed 10ms

Hide
# The seed stays open while its input is open (90 s here): it stands in for a running worker.
Type "export LIBERARR_HOME=${LIBERARR_DEMO_HOME:-/tmp/liberarr-demo} COLORTERM=truecolor LIBERARR_LANG=en"
Enter
Type "(sleep 90 | target/debug/examples/demo_home $LIBERARR_HOME >/dev/null 2>&1 &); sleep 3; clear"
Enter
Sleep 4s
Type "target/debug/liberarr tui"
Enter
Sleep 300ms
Show

Sleep 2500ms
Down
Sleep 1200ms
Down
Sleep 1200ms
Down
Sleep 2s
Up 3
Sleep 1s
Tab
Sleep 600ms
Right
Sleep 1800ms
Right
Sleep 1800ms
Right
Sleep 1s
Down 2
Sleep 500ms
Space
Sleep 1500ms
Type "?"
Sleep 2500ms
Escape
Sleep 1s

