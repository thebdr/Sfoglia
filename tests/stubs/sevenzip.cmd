@echo off
rem A stand-in for 7-Zip in the headless tests (C-015). Sfoglia calls it as
rem   (no arguments)                                                the version banner
rem   l -slt -y -sccUTF-8 -pPW -- ARCHIVE                           the listing: two files, a folder, a climbing entry
rem   e -so -y -sccUTF-8 -spd -pPW -- ARCHIVE ENTRY                  one entry's bytes on stdout
rem   x -oDIR -y -sccUTF-8 -scsUTF-8 -spd -i@LIST -pPW -- ARCHIVE    the entries the list file names, written
rem Every call must carry -sccUTF-8 and end its switches with -- (so a name is never a switch); x must name its
rem entries through the list file (the real 7-Zip would write everything otherwise); an archive named *secret*
rem opens with -psfoglia alone and answers as the real 7-Zip does otherwise.
if "%~1"=="" (
  echo 7-Zip 24.08 ^(x64^) : Copyright ^(c^) 1999-2024 Igor Pavlov : 2024-08-11
  exit /b 0
)
set "cmd=%~1"
set "pw="
set "list="
set "out="
set "scc=0"
set "dd=0"
set "archive="
set "entry="
shift /1
:switches
if "%~1"=="" goto run
if "%~1"=="--" (
  set "dd=1"
  shift /1
  goto names
)
set "a=%~1"
if "%a:~0,4%"=="-scc" set "scc=1"
if "%a:~0,2%"=="-p" set "pw=%a:~2%"
if "%a:~0,3%"=="-i@" set "list=%a:~3%"
if "%a:~0,2%"=="-o" set "out=%a:~2%"
shift /1
goto switches
:names
set "archive=%~1"
set "entry=%~2"
:run
if "%scc%"=="0" (
  echo stub: -sccUTF-8 missing 1>&2
  exit /b 7
)
if "%dd%"=="0" (
  echo stub: -- missing before the archive 1>&2
  exit /b 7
)
echo.%archive%| findstr /i "secret" >nul
if not errorlevel 1 if not "%pw%"=="sfoglia" (
  echo.
  echo ERROR: %archive% : Cannot open encrypted archive. Wrong password?
  exit /b 2
)
if "%cmd%"=="l" goto list
if "%cmd%"=="e" goto one
if "%cmd%"=="x" goto all
echo unknown command %cmd% 1>&2
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
if "%entry%"=="docs/readme.md" (
  echo # Readme from a rar
  exit /b 0
)
if "%entry%"=="big.bin" (
  echo 0123456789
  exit /b 0
)
echo stub: no such entry "%entry%" 1>&2
exit /b 2

:all
if "%list%"=="" (
  echo stub: no -i@ list file; the real 7-Zip would write every entry 1>&2
  exit /b 7
)
for /f "usebackq delims=" %%n in ("%list%") do call :write "%out%" "%%n"
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
  echo # Readme from a rar> "%~1\docs\readme.md"
  exit /b 0
)
if "%~2"=="../climb.txt" (
  rem As the real 7-Zip: the `..` stripped, the file written inside the folder.
  echo out> "%~1\climb.txt"
  exit /b 0
)
exit /b 0
