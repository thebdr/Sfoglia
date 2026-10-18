@echo off
rem A stand-in mutool that lists two pages and cannot draw the second (C-014's drawing failure).
if "%~1"=="-v" (
  echo mutool version 9.9
  exit /b 0
)
if "%~1"=="pages" (
  echo %~2:
  echo ^<page pagenum="1"^>
  echo ^<MediaBox l="0" b="0" r="612" t="792" /^>
  echo ^</page^>
  echo ^<page pagenum="2"^>
  echo ^<MediaBox l="0" b="0" r="612" t="792" /^>
  echo ^</page^>
  exit /b 0
)
if "%~1"=="draw" (
  echo draw %~9 at %~5>> "%~dp7calls.log"
  if "%~9"=="2" (
    echo error: cannot draw page 2 1>&2
    exit 1
  )
  copy /y "%~dp0..\fixtures\gradient.png" "%~7" >nul
  exit /b 0
)
echo unknown command %1 1>&2
exit /b 2
