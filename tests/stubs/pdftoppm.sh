#!/bin/sh
# A stand-in pdftoppm (C-014): -png -r DPI -f N -l N -singlefile PDF PREFIX writes PREFIX.png.
here=$(dirname "$0")
case "$1" in
  -v) echo "pdftoppm version 24.02.0" 1>&2 ;;
  -png)
    echo "draw $5 at $3" >> "$(dirname "${10}")/calls.log"
    cp "$here/../fixtures/dot.png" "${10}.png" ;;
  *) echo "unknown arguments $*" 1>&2; exit 2 ;;
esac
