@echo off
setlocal
call "%~dp0cargo.cmd" run -p lotus-app -- %*
exit /b %ERRORLEVEL%
