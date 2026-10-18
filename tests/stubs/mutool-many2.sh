#!/bin/sh
# A stand-in mutool with eight Letter pages whose second page never draws (C-014: a complaint that stays).
here=$(dirname "$0")
case "$1" in
  -v) echo "mutool version 9.9" ;;
  pages)
    echo "$2:"
    for p in 1 2 3 4 5 6 7 8; do printf '%s\n' "<page pagenum=\"$p\">" '<MediaBox l="0" b="0" r="612" t="792" />' '</page>'; done ;;
  draw)
    echo "draw $9 at $5" >> "$(dirname "$7")/calls.log"
    if [ "$9" = "2" ]; then echo "error: cannot draw page 2" 1>&2; exit 1; fi
    cp "$here/../fixtures/gradient.png" "$7" ;;
  *) echo "unknown command $1" 1>&2; exit 2 ;;
esac
