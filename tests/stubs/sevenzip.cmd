@echo off
rem A stand-in for 7-Zip in the headless tests (C-015). The calls Sfoglia makes:
rem   (no arguments)                                    the version banner
rem   l -slt -y -sccUTF-8 -pPW ARCHIVE                  the listing: two files, a folder and a climbing entry
rem   e -so -y -sccUTF-8 -pPW ARCHIVE ENTRY             the entry's bytes on stdout
rem   x -oFOLDER -y -sccUTF-8 -pPW ARCHIVE [ENTRY...]   the named entries written (all when none is named)
rem Every call must carry -sccUTF-8 (the names come back in UTF-8); an archive whose name holds "secret" opens
rem with -psfoglia alone and answers as the real 7-Zip does otherwise.
if "%~1"=="" (
  echo 7-Zip 24.08 ^(x64^) : Copyright ^(c^) 1999-2024 Igor Pavlov : 2024-08-11
  exit /b 0
)
if not "%~4"=="-sccUTF-8" (
  echo stub: -sccUTF-8 missing, got "%~4" 1>&2
  exit /b 7
)
set "pw=%~5"
set "archive=%~6"
echo.%archive%| findstr /i "secret" >nul
if not errorlevel 1 if not "%pw%"=="-psfoglia" (
  echo.
  echo ERROR: %archive% : Cannot open encrypted archive. Wrong password?
  exit /b 2
)
if "%~1"=="l" goto list
if "%~1"=="e" goto one
if "%~1"=="x" goto all
echo unknown command %1 1>&2
exit /b 2

:list
echo.
echo 7-Zip 24.08 ^(x64^) : Copyright ^(c^) 1999-2024 Igor Pavlov : 2024-08-11
echo.
echo Listing archive: %archive%
echo.
echo --
echo Path = %archive%
echo Type = Rar5
echo.
type "%~dp0sevenzip-listing.txt"
exit /b 0

:one
if "%~7"=="docs/readme.md" (
  echo # Readme from a rar
  exit /b 0
)
if "%~7"=="big.bin" (
  echo 0123456789
  exit /b 0
)
echo stub: no such entry "%~7" 1>&2
exit /b 2

:all
set "out=%~2"
set "out=%out:~2%"
shift & shift & shift & shift & shift & shift
if "%~1"=="" (
  call :write "%out%" docs/readme.md
  call :write "%out%" big.bin
  call :write "%out%" docs
  echo Everything is Ok
  exit /b 0
)
:loop
if "%~1"=="" goto done
call :write "%out%" "%~1"
shift
goto loop
:done
echo Everything is Ok
exit /b 0

:write
if "%~2"=="docs/readme.md" (
  mkdir "%~1\docs" 2>nul
  echo # Readme from a rar> "%~1\docs\readme.md"
  exit /b 0
)
if "%~2"=="big.bin" (
  echo 0123456789> "%~1\big.bin"
  exit /b 0
)
if "%~2"=="docs" (
  mkdir "%~1\docs" 2>nul
  exit /b 0
)
if "%~2"=="../climb.txt" (
  echo out> "%~1\..\climb.txt"
  exit /b 0
)
exit /b 0
