#!/bin/sh
# A stand-in for 7-Zip in the headless tests (C-015). The calls Sfoglia makes:
#   (no arguments)                                    the version banner
#   l -slt -y -sccUTF-8 -pPW ARCHIVE                  the listing: two files, a folder and a climbing entry
#   e -so -y -sccUTF-8 -pPW ARCHIVE ENTRY             the entry's bytes on stdout
#   x -oFOLDER -y -sccUTF-8 -pPW ARCHIVE [ENTRY...]   the named entries written (all when none is named)
# Every call must carry -sccUTF-8 (the names come back in UTF-8); an archive whose name holds "secret" opens
# with -psfoglia alone and answers as the real 7-Zip does otherwise.
here=$(dirname "$0")
if [ "$#" -eq 0 ]; then
  echo "7-Zip 24.08 (x64) : Copyright (c) 1999-2024 Igor Pavlov : 2024-08-11"
  exit 0
fi
if [ "$4" != "-sccUTF-8" ]; then
  echo "stub: -sccUTF-8 missing, got \"$4\"" 1>&2
  exit 7
fi
pw=$5
archive=$6
case "$archive" in
  *secret*|*Secret*|*SECRET*)
    if [ "$pw" != "-psfoglia" ]; then
      echo
      echo "ERROR: $archive : Cannot open encrypted archive. Wrong password?"
      exit 2
    fi ;;
esac
write() {
  case "$2" in
    docs/readme.md) mkdir -p "$1/docs"; echo "# Readme from a rar" > "$1/docs/readme.md" ;;
    big.bin) echo "0123456789" > "$1/big.bin" ;;
    docs) mkdir -p "$1/docs" ;;
    ../climb.txt) echo "out" > "$1/../climb.txt" ;;
  esac
}
case "$1" in
  l)
    echo
    echo "7-Zip 24.08 (x64) : Copyright (c) 1999-2024 Igor Pavlov : 2024-08-11"
    echo
    echo "Listing archive: $archive"
    echo
    echo "--"
    echo "Path = $archive"
    echo "Type = Rar5"
    echo
    cat "$here/sevenzip-listing.txt"
    exit 0 ;;
  e)
    case "$7" in
      docs/readme.md) echo "# Readme from a rar"; exit 0 ;;
      big.bin) echo "0123456789"; exit 0 ;;
    esac
    echo "stub: no such entry \"$7\"" 1>&2
    exit 2 ;;
  x)
    out=${2#-o}
    shift 6
    if [ "$#" -eq 0 ]; then
      set -- docs/readme.md big.bin docs
    fi
    for entry in "$@"; do
      write "$out" "$entry"
    done
    echo "Everything is Ok"
    exit 0 ;;
esac
echo "unknown command $1" 1>&2
exit 2
