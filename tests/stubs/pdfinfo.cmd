@echo off
rem A stand-in pdfinfo (C-014): two pages, the second one landscape by its rotation.
if "%~1"=="-v" (
  echo pdfinfo version 24.02.0 1>&2
  exit /b 0
)
echo Title:          stub
echo Pages:          2
echo Page size:      612 x 792 pts (letter)
echo Page    1 size: 612 x 792 pts (letter)
echo Page    1 rot:  0
echo Page    2 size: 612 x 792 pts (letter)
echo Page    2 rot:  90
exit /b 0
