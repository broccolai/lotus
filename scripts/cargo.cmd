@echo off
setlocal
set "LOTUS_CARGO=%~dp0..\.cargo-home\bin\cargo.exe"
if exist "%LOTUS_CARGO%" (
    set "CARGO_HOME=%~dp0..\.cargo-home"
    set "RUSTUP_HOME=%~dp0..\.rustup-home"
) else (
    set "LOTUS_CARGO=cargo.exe"
)
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if exist "%VSWHERE%" for /f "usebackq tokens=*" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "LOTUS_BUILD_TOOLS=%%i"
if defined LOTUS_BUILD_TOOLS call "%LOTUS_BUILD_TOOLS%\VC\Auxiliary\Build\vcvars64.bat" >nul
"%LOTUS_CARGO%" %*
exit /b %ERRORLEVEL%
