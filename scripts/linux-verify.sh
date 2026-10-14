#!/usr/bin/env bash
# Build, test and run Sfoglia on the Linux VM; capture the real window on the X11 desktop.
# Writes ~/sfoglia-linux.log, ~/sfoglia-linux.png and ~/sfoglia-linux.result (one line of facts).
set -u
export PATH="$HOME/.cargo/bin:$PATH"
LOG="$HOME/sfoglia-linux.log"
RESULT="$HOME/sfoglia-linux.result"
PNG="$HOME/sfoglia-linux.png"
: > "$LOG"; : > "$RESULT"; rm -f "$PNG"
log() { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" | tee -a "$LOG"; }

log "== clone or update =="
if [ -d "$HOME/Sfoglia/.git" ]; then
  git -C "$HOME/Sfoglia" fetch -q origin && git -C "$HOME/Sfoglia" reset -q --hard origin/main
else
  git clone -q https://github.com/thebdr/Sfoglia "$HOME/Sfoglia"
fi
cd "$HOME/Sfoglia" || exit 1
log "commit $(git rev-parse --short HEAD)"

log "== cargo build --locked =="
cargo build --locked >>"$LOG" 2>&1; BUILD=$?
log "build exit $BUILD"

log "== cargo test --locked =="
cargo test --locked >>"$LOG" 2>&1; TEST=$?
log "test exit $TEST"
grep -E "^test result|^test .*\.\.\." "$LOG" | tail -20 >>"$RESULT.tests" 2>/dev/null || true

log "== run the app on the desktop =="
export DISPLAY=:0
export XAUTHORITY="${XAUTHORITY:-$HOME/.Xauthority}"
if ! xdpyinfo >/dev/null 2>&1; then
  XA=$(ps -eo args | grep -oE 'Xorg.* -auth [^ ]+' | sed 's/.* -auth //' | head -1)
  [ -n "$XA" ] && export XAUTHORITY="$XA"
fi
log "DISPLAY=$DISPLAY XAUTHORITY=$XAUTHORITY xdpyinfo=$(xdpyinfo >/dev/null 2>&1 && echo ok || echo FAIL)"
./target/debug/sfoglia >"$HOME/sfoglia-app.log" 2>&1 &
APP=$!
WID=""
for i in $(seq 1 60); do
  WID=$(xdotool search --onlyvisible --name '^Sfoglia$' 2>/dev/null | head -1)
  [ -n "$WID" ] && break
  sleep 0.5
done
if [ -n "$WID" ]; then
  sleep 1.5
  TITLE=$(xdotool getwindowname "$WID" 2>/dev/null)
  GEOM=$(xdotool getwindowgeometry --shell "$WID" 2>/dev/null | tr '\n' ' ')
  xdotool windowactivate --sync "$WID" 2>/dev/null; sleep 0.5
  scrot -o -u "$PNG" 2>>"$LOG" || scrot -o "$PNG" 2>>"$LOG"
  log "window id=$WID title='$TITLE' $GEOM png=$( [ -s "$PNG" ] && stat -c %s "$PNG" || echo missing)"
else
  log "no window titled Sfoglia appeared within 30 s"
  tail -5 "$HOME/sfoglia-app.log" >>"$LOG"
fi
kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null
APPEXIT=$?
log "app stopped (exit $APPEXIT)"
printf 'build=%s test=%s window=%s title=%s png=%s\n' "$BUILD" "$TEST" "${WID:-none}" "${TITLE:-none}" "$( [ -s "$PNG" ] && echo yes || echo no)" > "$RESULT"
cat "$RESULT"
