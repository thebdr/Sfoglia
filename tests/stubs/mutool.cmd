@echo off
rem A stand-in for mutool in the headless tests (C-014): a version, two pages, a fixture PNG per page.
if "%~1"=="-v" (
  echo mutool version 9.9
  exit /b 0
)
if "%~1"=="pages" (
  echo %~2:
  echo ^<page pagenum="1"^>
  echo ^<MediaBox l="0" b="0" r="612" t="792" /^>
  echo ^<Rotate v="0" /^>
  echo ^</page^>
  echo ^<page pagenum="2"^>
  echo ^<MediaBox l="0" b="0" r="400" t="200" /^>
  echo ^<Rotate v="0" /^>
  echo ^</page^>
  exit /b 0
)
if "%~1"=="draw" (
  rem draw -F png -r DPI -o OUT PDF PAGE
  if "%~9"=="2" (
    copy /y "%~dp0..\fixtures\dot.png" "%~7" >nul
  ) else (
    copy /y "%~dp0..\fixtures\gradient.png" "%~7" >nul
  )
  exit /b 0
)
echo unknown command %1 1>&2
exit /b 2
