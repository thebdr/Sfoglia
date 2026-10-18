@echo off
rem A stand-in for 7-Zip in the headless tests (C-015): a version banner, a listing of two entries and a
rem folder for any archive, the entries' bytes on -so, and the files written on x.
if "%~1"=="" (
  echo 7-Zip 24.08 ^(x64^) : Copyright ^(c^) 1999-2024 Igor Pavlov : 2024-08-11
  exit /b 0
)
if "%~1"=="l" (
  echo.
  echo 7-Zip 24.08 ^(x64^) : Copyright ^(c^) 1999-2024 Igor Pavlov : 2024-08-11
  echo.
  echo Listing archive: %~4
  echo.
  echo --
  echo Path = %~4
  echo Type = Rar5
  echo.
  type "%~dp0sevenzip-listing.txt"
  exit /b 0
)
if "%~1"=="e" (
  rem e -so -y -sccUTF-8 -pPASSWORD ARCHIVE ENTRY
  if "%~7"=="docs/readme.md" (
    echo # Readme from a rar
    exit /b 0
  )
  if "%~6"=="docs/readme.md" (
    echo # Readme from a rar
    exit /b 0
  )
  echo stub: no such entry 1>&2
  exit /b 2
)
if "%~1"=="x" (
  rem x -oFOLDER -y [-pPASSWORD] ARCHIVE [ENTRY...]
  set "out=%~2"
  setlocal enabledelayedexpansion
  set "out=!out:~2!"
  mkdir "!out!\docs" 2>nul
  echo # Readme from a rar> "!out!\docs\readme.md"
  echo 0123456789> "!out!\big.bin"
  endlocal
  echo Everything is Ok
  exit /b 0
)
echo unknown command %1 1>&2
exit /b 2
