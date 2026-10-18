@echo off
rem A mutool that is installed but cannot read the file (C-014's failure path).
if "%~1"=="-v" (
  echo mutool version 9.9
  exit /b 0
)
echo error: cannot open document 1>&2
echo error: a second line the status row never shows 1>&2
exit /b 1
