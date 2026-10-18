@echo off
rem A stand-in pdftoppm (C-014): -png -r DPI -f N -l N -singlefile PDF PREFIX writes PREFIX.png.
if "%~1"=="-v" (
  echo pdftoppm version 24.02.0 1>&2
  exit /b 0
)
if not "%~1"=="-png" (
  echo unknown arguments %* 1>&2
  exit 2
)
set "dpi=%~3"
set "page=%~5"
rem the prefix is the tenth argument: one shift (of the arguments alone, %0 kept) brings it to %9
shift /1
echo draw %page% at %dpi%>> "%~dp9calls.log"
copy /y "%~dp0..\fixtures\dot.png" "%~9.png" >nul
exit /b 0
