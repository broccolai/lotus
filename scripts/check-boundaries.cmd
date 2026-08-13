@echo off
setlocal
set "SAFE_SOURCES=crates\lotus-app\src crates\lotus-core\src crates\lotus-dock\src crates\lotus-search\src crates\lotus-settings\src crates\lotus-switcher\src crates\lotus-ui\src"
rg --pcre2 -n "(?<!lotus_)\bwindows::|\bunsafe\s*(\{|fn\b|impl\b|trait\b|extern\b)" %SAFE_SOURCES%
if %ERRORLEVEL% EQU 0 (
    echo Raw Windows or unsafe code is only allowed in crates\lotus-windows. 1>&2
    exit /b 1
)
if %ERRORLEVEL% GTR 1 exit /b %ERRORLEVEL%

rg -n "^windows\.workspace" crates\lotus-app\Cargo.toml crates\lotus-core\Cargo.toml crates\lotus-dock\Cargo.toml crates\lotus-search\Cargo.toml crates\lotus-settings\Cargo.toml crates\lotus-switcher\Cargo.toml crates\lotus-ui\Cargo.toml
if %ERRORLEVEL% EQU 0 (
    echo The windows dependency is only allowed in crates\lotus-windows. 1>&2
    exit /b 1
)
if %ERRORLEVEL% GTR 1 exit /b %ERRORLEVEL%
exit /b 0
