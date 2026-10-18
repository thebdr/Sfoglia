@echo off
rem A stand-in that runs until it is ended (C-014: a hosted process ends with its tab).
:loop
ping -n 2 127.0.0.1 >nul
goto loop
