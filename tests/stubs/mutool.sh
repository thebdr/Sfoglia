#!/bin/sh
# A stand-in for mutool in the headless tests (C-014): a version, two pages, a fixture PNG per page.
here=$(dirname "$0")
case "$1" in
  -v) echo "mutool version 9.9" ;;
  pages)
    printf '%s\n' "$2:" '<page pagenum="1">' '<MediaBox l="0" b="0" r="612" t="792" />' '<Rotate v="0" />' '</page>' \
      '<page pagenum="2">' '<MediaBox l="0" b="0" r="400" t="200" />' '<Rotate v="0" />' '</page>' ;;
  draw)
    # draw -F png -r DPI -o OUT PDF PAGE
    if [ "$9" = "2" ]; then cp "$here/../fixtures/dot.png" "$7"; else cp "$here/../fixtures/gradient.png" "$7"; fi ;;
  *) echo "unknown command $1" 1>&2; exit 2 ;;
esac
