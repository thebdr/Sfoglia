#!/bin/sh
# A stand-in for 7-Zip in the headless tests (C-015). Sfoglia calls it as
#   (no arguments)                                                the version banner
#   l -slt -y -sccUTF-8 -pPW -- ARCHIVE                           the listing: two files, a folder, a climbing entry
#   e -so -y -sccUTF-8 -spd -pPW -- ARCHIVE ENTRY                  one entry's bytes on stdout
#   x -oDIR -y -sccUTF-8 -scsUTF-8 -spd -i@LIST -pPW -- ARCHIVE    the entries the list file names, written
# Every call must carry -sccUTF-8 and end its switches with -- (so a name is never a switch); x must name its
# entries through the list file (the real 7-Zip would write everything otherwise); an archive named *secret*
# opens with -psfoglia alone and answers as the real 7-Zip does otherwise.
here=$(dirname "$0")
if [ "$#" -eq 0 ]; then
  echo "7-Zip 24.08 (x64) : Copyright (c) 1999-2024 Igor Pavlov : 2024-08-11"
  exit 0
fi
cmd=$1
shift
pw=; list=; out=; scc=0; dd=0; archive=; entry=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --) dd=1; shift; archive=$1; entry=$2; break ;;
    -scc*) scc=1 ;;
    -i@*) list=${1#-i@} ;;
    -o*) out=${1#-o} ;;
    -p*) pw=${1#-p} ;;
  esac
  shift
done
if [ "$scc" != "1" ]; then
  echo "stub: -sccUTF-8 missing" 1>&2
  exit 7
fi
if [ "$dd" != "1" ]; then
  echo "stub: -- missing before the archive" 1>&2
  exit 7
fi
case "$archive" in
  *secret*|*Secret*|*SECRET*)
    if [ "$pw" != "sfoglia" ]; then
      echo
      echo "ERROR: $archive : Cannot open encrypted archive. Wrong password?"
      exit 2
    fi ;;
esac
write() {
  case "$2" in
    docs/readme.md) mkdir -p "$1/docs"; echo "# Readme from a rar" > "$1/docs/readme.md" ;;
    big.bin) echo "0123456789" > "$1/big.bin" ;;
    docs) mkdir -p "$1/docs"; echo "# Readme from a rar" > "$1/docs/readme.md" ;;
    # As the real 7-Zip: the `..` stripped, the file written inside the folder.
    ../climb.txt) echo "out" > "$1/climb.txt" ;;
  esac
}
case "$cmd" in
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
    case "$entry" in
      docs/readme.md) echo "# Readme from a rar"; exit 0 ;;
      big.bin) echo "0123456789"; exit 0 ;;
    esac
    echo "stub: no such entry \"$entry\"" 1>&2
    exit 2 ;;
  x)
    if [ -z "$list" ]; then
      echo "stub: no -i@ list file; the real 7-Zip would write every entry" 1>&2
      exit 7
    fi
    while IFS= read -r name || [ -n "$name" ]; do
      [ -n "$name" ] && write "$out" "$name"
    done < "$list"
    echo "Everything is Ok"
    exit 0 ;;
esac
echo "unknown command $cmd" 1>&2
exit 2
