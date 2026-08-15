@echo off
setlocal
call "%~dp0check-boundaries.cmd"
if errorlevel 1 exit /b %ERRORLEVEL%
call "%~dp0cargo.cmd" fmt --all -- --check
if errorlevel 1 exit /b %ERRORLEVEL%
call "%~dp0cargo.cmd" clippy --workspace --all-targets --all-features -- -D warnings
if errorlevel 1 exit /b %ERRORLEVEL%
call "%~dp0cargo.cmd" build -p lotus-app -p lotus-shell-bridge
exit /b %ERRORLEVEL%
