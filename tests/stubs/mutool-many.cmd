@echo off
rem A stand-in mutool with eight Letter pages (C-014): the drawings are asked for around the visible ones.
if "%~1"=="-v" (
  echo mutool version 9.9
  exit /b 0
)
if "%~1"=="pages" (
  echo %~2:
  for %%p in (1 2 3 4 5 6 7 8) do (
    echo ^<page pagenum="%%p"^>
    echo ^<MediaBox l="0" b="0" r="612" t="792" /^>
    echo ^</page^>
  )
  exit /b 0
)
if "%~1"=="draw" (
  echo draw %~9 at %~5>> "%~dp7calls.log"
  copy /y "%~dp0..\fixtures\gradient.png" "%~7" >nul
  exit /b 0
)
echo unknown command %1 1>&2
exit /b 2
