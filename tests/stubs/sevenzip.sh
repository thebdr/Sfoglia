#!/bin/sh
# A stand-in for 7-Zip in the headless tests (C-015): a version banner, a listing of two entries and a
# folder for any archive, the entries' bytes on -so, and the files written on x.
here=$(dirname "$0")
case "$1" in
  "")
    echo "7-Zip 24.08 (x64) : Copyright (c) 1999-2024 Igor Pavlov : 2024-08-11"
    exit 0 ;;
  l)
    echo
    echo "7-Zip 24.08 (x64) : Copyright (c) 1999-2024 Igor Pavlov : 2024-08-11"
    echo
    echo "Listing archive: $4"
    echo
    echo "--"
    echo "Path = $4"
    echo "Type = Rar5"
    echo
    cat "$here/sevenzip-listing.txt"
    exit 0 ;;
  e)
    # e -so -y [-pPASSWORD] ARCHIVE ENTRY
    for arg in "$@"; do
      if [ "$arg" = "docs/readme.md" ]; then
        echo "# Readme from a rar"
        exit 0
      fi
    done
    echo "stub: no such entry" 1>&2
    exit 2 ;;
  x)
    # x -oFOLDER -y [-pPASSWORD] ARCHIVE [ENTRY...]
    out=${2#-o}
    mkdir -p "$out/docs"
    echo "# Readme from a rar" > "$out/docs/readme.md"
    echo "0123456789" > "$out/big.bin"
    echo "Everything is Ok"
    exit 0 ;;
esac
echo "unknown command $1" 1>&2
exit 2
