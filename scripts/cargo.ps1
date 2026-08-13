param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $CargoArguments
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$localCargoHome = Join-Path $projectRoot ".cargo-home"
$localRustupHome = Join-Path $projectRoot ".rustup-home"
$localCargo = Join-Path $localCargoHome "bin\cargo.exe"

if (Test-Path -LiteralPath $localCargo) {
    $env:CARGO_HOME = $localCargoHome
    $env:RUSTUP_HOME = $localRustupHome
    $cargo = $localCargo
} else {
    $cargo = "cargo.exe"
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path -LiteralPath $vswhere) {
    $buildTools = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($buildTools) {
        $developerShell = Join-Path $buildTools "Common7\Tools\Launch-VsDevShell.ps1"
        & $developerShell -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
    }
}

& $cargo @CargoArguments
exit $LASTEXITCODE
