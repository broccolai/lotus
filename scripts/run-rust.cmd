@echo off
setlocal
call "%~dp0cargo.cmd" build -p lotus-shell-bridge -p lotus-explorer-bridge
if errorlevel 1 exit /b %ERRORLEVEL%
call "%~dp0cargo.cmd" run -p lotus-app -- %*
exit /b %ERRORLEVEL%
