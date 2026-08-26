[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

function Invoke-CargoStep {
    param(
        [string]$Name,
        [string[]]$Arguments
    )

    Write-Host "[$Name] cargo $($Arguments -join ' ')"
    & cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Name failed with exit code $LASTEXITCODE"
    }
}

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Push-Location $repoRoot

try {
    & "$PSScriptRoot\check-architecture.ps1"
    Invoke-CargoStep 'format' @('fmt', '--all', '--', '--check')
    Invoke-CargoStep 'clippy' @('clippy', '--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings')
    Invoke-CargoStep 'test' @('test', '--workspace', '--all-features', '--locked')
    Invoke-CargoStep 'build' @('build', '--locked', '-p', 'lotus-app', '-p', 'lotus-shell-bridge', '-p', 'lotus-explorer-bridge')
}
finally {
    Pop-Location
}

Write-Host 'Windows workspace verification passed.'
