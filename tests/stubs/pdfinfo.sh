#!/bin/sh
# A stand-in pdfinfo (C-014): two pages, the second one landscape by its rotation.
if [ "$1" = "-v" ]; then echo "pdfinfo version 24.02.0" 1>&2; exit 0; fi
printf '%s\n' 'Title:          stub' 'Pages:          2' 'Page size:      612 x 792 pts (letter)' \
  'Page    1 size: 612 x 792 pts (letter)' 'Page    1 rot:  0' 'Page    2 size: 612 x 792 pts (letter)' 'Page    2 rot:  90'
