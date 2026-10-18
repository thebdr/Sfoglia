#!/bin/sh
# A mutool that is installed but cannot read the file (C-014's failure path).
if [ "$1" = "-v" ]; then echo "mutool version 9.9"; exit 0; fi
echo "error: cannot open document" 1>&2
exit 1
