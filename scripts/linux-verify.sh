#!/usr/bin/env bash
# Build, test and run Sfoglia on the Linux VM; capture the real window on the X11 desktop.
#
#   bash linux-verify.sh [BRANCH] [FILE...]
#
# BRANCH defaults to main. FILEs are paths inside the clone (for example
# tests/fixtures/hello.txt) handed to the app on its command line. When files are given
# the script also drives the Open dialog with Ctrl+O and types the path of a third fixture.
# Optional steps, enabled by environment variables:
#   TOGGLE_THEME=1   press Ctrl+Shift+T and capture ~/sfoglia-linux-dark.png, then press it again
#   SCROLL_END=1     click into the text and press Ctrl+End, capture ~/sfoglia-linux-end.png
#   RULES_PANEL=1    press Ctrl+Shift+H (the highlighting rules panel) and capture
#                    ~/sfoglia-linux-rules.png; the panel stays open for the later captures
#   HEX_SEARCH=text  in the hex view: press Ctrl+Shift+F, type the text, press Return and
#                    capture ~/sfoglia-linux-hex.png with the hit marked
#   HEX_BYTES=1      with HEX_SEARCH: press Ctrl+M first, so the text is hexadecimal pairs
#   DATA_WALK=1      in the data tree: press Down three times and Return (opens the node), then
#                    Ctrl+Shift+F, type "deep", Return, capture ~/sfoglia-linux-data.png
#   EDIT=1           click into the text, type "X" (the tab shows its marker), then append a
#                    line to the first FILE from outside and capture ~/sfoglia-linux-edit.png
#                    once the changed-on-disk bar has appeared
# Writes ~/sfoglia-linux.log, ~/sfoglia-linux.png (after launch), ~/sfoglia-linux-dialog.png
# (after the dialog, when driven) and ~/sfoglia-linux.result (one line of facts).
set -u
export PATH="$HOME/.cargo/bin:$PATH"
BRANCH="${1:-main}"; shift || true
LOG="$HOME/sfoglia-linux.log"
RESULT="$HOME/sfoglia-linux.result"
PNG="$HOME/sfoglia-linux.png"
PNG2="$HOME/sfoglia-linux-dialog.png"
PNGDARK="$HOME/sfoglia-linux-dark.png"
PNGEND="$HOME/sfoglia-linux-end.png"
PNGRULES="$HOME/sfoglia-linux-rules.png"
PNGHEX="$HOME/sfoglia-linux-hex.png"
PNGEDIT="$HOME/sfoglia-linux-edit.png"
PNGDATA="$HOME/sfoglia-linux-data.png"
: > "$LOG"; : > "$RESULT"; rm -f "$PNG" "$PNG2" "$PNGDARK" "$PNGEND" "$PNGRULES" "$PNGHEX" "$PNGEDIT" "$PNGDATA"
log() { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" | tee -a "$LOG"; }
shot() { scrot -o -u "$1" 2>>"$LOG" || scrot -o "$1" 2>>"$LOG"; log "$2 png=$( [ -s "$1" ] && stat -c %s "$1" || echo missing)"; }

log "== clone or update ($BRANCH) =="
if [ -d "$HOME/Sfoglia/.git" ]; then
  git -C "$HOME/Sfoglia" fetch -q origin "$BRANCH" && git -C "$HOME/Sfoglia" reset -q --hard "origin/$BRANCH"
else
  git clone -q --branch "$BRANCH" https://github.com/thebdr/Sfoglia "$HOME/Sfoglia"
fi
cd "$HOME/Sfoglia" || exit 1
log "commit $(git rev-parse --short HEAD)"

log "== cargo build --locked =="
cargo build --locked >>"$LOG" 2>&1; BUILD=$?
log "build exit $BUILD"

log "== cargo test --locked =="
cargo test --locked >>"$LOG" 2>&1; TEST=$?
log "test exit $TEST"
grep -E "300k lines" "$LOG" | tail -1 | tee -a "$RESULT.timing" >/dev/null || true

log "== run the app on the desktop =="
export DISPLAY=:0
export XAUTHORITY="${XAUTHORITY:-$HOME/.Xauthority}"
if ! xdpyinfo >/dev/null 2>&1; then
  XA=$(ps -eo args | grep -oE 'Xorg.* -auth [^ ]+' | sed 's/.* -auth //' | head -1)
  [ -n "$XA" ] && export XAUTHORITY="$XA"
fi
log "DISPLAY=$DISPLAY XAUTHORITY=$XAUTHORITY xdpyinfo=$(xdpyinfo >/dev/null 2>&1 && echo ok || echo FAIL) args: $*"
./target/debug/sfoglia "$@" >"$HOME/sfoglia-app.log" 2>&1 &
APP=$!
WID=""
for i in $(seq 1 60); do
  WID=$(xdotool search --onlyvisible --name '^Sfoglia$' 2>/dev/null | head -1)
  [ -n "$WID" ] && break
  sleep 0.5
done
TITLE=""; GEOM=""; DIALOG=""
if [ -n "$WID" ]; then
  sleep 2.5
  TITLE=$(xdotool getwindowname "$WID" 2>/dev/null)
  GEOM=$(xdotool getwindowgeometry --shell "$WID" 2>/dev/null | tr '\n' ' ')
  xdotool windowactivate --sync "$WID" 2>/dev/null; sleep 0.5
  shot "$PNG" "window id=$WID title='$TITLE' $GEOM"
  if [ "${RULES_PANEL:-0}" = "1" ]; then
    xdotool key --window "$WID" ctrl+shift+h; sleep 1.5
    shot "$PNGRULES" "after Ctrl+Shift+H"
  fi
  if [ -n "${HEX_SEARCH:-}" ]; then
    # The hex view holds focus when a binary opens; Ctrl+Shift+F focuses its search field,
    # and HEX_BYTES=1 presses Ctrl+M first so the text is read as hexadecimal pairs.
    [ "${HEX_BYTES:-0}" = "1" ] && { xdotool key --window "$WID" ctrl+m; sleep 0.3; }
    xdotool key --window "$WID" ctrl+shift+f; sleep 0.5
    xdotool type --delay 20 "$HEX_SEARCH"; sleep 0.3
    xdotool key Return; sleep 1.5
    shot "$PNGHEX" "after Ctrl+Shift+F, '$HEX_SEARCH', Return"
  fi
  if [ "${DATA_WALK:-0}" = "1" ]; then
    # The data tree (C-010): walk down to the fourth row, open it, then find "deep".
    xdotool key --window "$WID" Down Down Down Return; sleep 0.8
    xdotool key --window "$WID" ctrl+shift+f; sleep 0.5
    xdotool type --delay 20 "deep"; sleep 0.3
    xdotool key Return; sleep 1.5
    shot "$PNGDATA" "after Down x3, Return, Ctrl+Shift+F, deep, Return"
  fi
  if [ "${EDIT:-0}" = "1" ] && [ "$#" -gt 0 ]; then
    eval "$GEOM"
    xdotool mousemove $((X + WIDTH / 3)) $((Y + HEIGHT / 3)) click 1; sleep 0.5
    xdotool type --delay 40 "X"; sleep 0.5
    printf 'changed elsewhere\n' >> "$HOME/Sfoglia/$1"
    sleep 2.5
    shot "$PNGEDIT" "after typing X and a change on disk to $1"
  fi
  if [ "${TOGGLE_THEME:-0}" = "1" ]; then
    xdotool key --window "$WID" ctrl+shift+t; sleep 1.5
    shot "$PNGDARK" "after Ctrl+Shift+T"
    xdotool key --window "$WID" ctrl+shift+t; sleep 1
  fi
  if [ "${SCROLL_END:-0}" = "1" ]; then
    eval "$GEOM"
    xdotool mousemove $((X + WIDTH / 2)) $((Y + HEIGHT / 2)) click 1; sleep 0.5
    xdotool key --window "$WID" ctrl+End; sleep 1.5
    shot "$PNGEND" "after click and Ctrl+End"
  fi
  if [ "$#" -gt 0 ]; then
    log "== Ctrl+O: the Open dialog =="
    xdotool key --window "$WID" ctrl+o; sleep 2.5
    DIALOG=$(xdotool search --onlyvisible --name '^Open' 2>/dev/null | head -1)
    [ -z "$DIALOG" ] && DIALOG=$(xdotool getactivewindow 2>/dev/null)
    DTITLE=$(xdotool getwindowname "$DIALOG" 2>/dev/null)
    log "dialog id=$DIALOG title='$DTITLE'"
    if [ -n "$DIALOG" ] && [ "$DIALOG" != "$WID" ]; then
      # GTK file chooser: Ctrl+L focuses the location entry; type an absolute path and confirm.
      xdotool key --window "$DIALOG" ctrl+l; sleep 0.5
      xdotool type --delay 20 "$HOME/Sfoglia/tests/fixtures/data.json"; sleep 0.3
      xdotool key Return; sleep 2.5
      xdotool windowactivate --sync "$WID" 2>/dev/null; sleep 0.5
      shot "$PNG2" "after dialog"
    fi
  fi
else
  log "no window titled Sfoglia appeared within 30 s"
  tail -5 "$HOME/sfoglia-app.log" >>"$LOG"
fi
kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null
log "app stopped"
printf 'build=%s test=%s window=%s title=%s png=%s dialog=%s png2=%s dark=%s end=%s rules=%s hex=%s edit=%s\n' "$BUILD" "$TEST" "${WID:-none}" "${TITLE:-none}" \
  "$( [ -s "$PNG" ] && echo yes || echo no)" "${DIALOG:-none}" "$( [ -s "$PNG2" ] && echo yes || echo no)" \
  "$( [ -s "$PNGDARK" ] && echo yes || echo no)" "$( [ -s "$PNGEND" ] && echo yes || echo no)" \
  "$( [ -s "$PNGRULES" ] && echo yes || echo no)" "$( [ -s "$PNGHEX" ] && echo yes || echo no)" \
  "$( [ -s "$PNGEDIT" ] && echo yes || echo no)" "$( [ -s "$PNGDATA" ] && echo yes || echo no)" > "$RESULT"
cat "$RESULT"
