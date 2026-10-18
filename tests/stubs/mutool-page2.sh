#!/bin/sh
# A stand-in mutool that lists two pages and cannot draw the second (C-014's drawing failure).
here=$(dirname "$0")
case "$1" in
  -v) echo "mutool version 9.9" ;;
  pages)
    printf '%s\n' "$2:" '<page pagenum="1">' '<MediaBox l="0" b="0" r="612" t="792" />' '</page>' \
      '<page pagenum="2">' '<MediaBox l="0" b="0" r="612" t="792" />' '</page>' ;;
  draw)
    echo "draw $9 at $5" >> "$(dirname "$7")/calls.log"
    if [ "$9" = "2" ]; then echo "error: cannot draw page 2" 1>&2; exit 1; fi
    cp "$here/../fixtures/gradient.png" "$7" ;;
  *) echo "unknown command $1" 1>&2; exit 2 ;;
esac
