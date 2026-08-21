[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $InputPath,
    [string] $OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $InputPath -PathType Leaf)) { throw "InputPath does not exist: $InputPath" }
$lines = Get-Content -LiteralPath $InputPath
$dataLines = $lines | Where-Object { $_ -and -not $_.StartsWith("#") }
if ($dataLines.Count -lt 2) { throw "The capture contains no samples." }
$rows = $dataLines | ConvertFrom-Csv
$groups = $rows | Group-Object ProcessName
$summary = foreach ($group in $groups) {
    $cpu = @($group.Group | Where-Object { $_.CpuPercent -ne "" } | ForEach-Object { [double]$_.CpuPercent })
    $private = @($group.Group | Where-Object { $_.PrivateBytes -ne "" } | ForEach-Object { [double]$_.PrivateBytes })
    $handles = @($group.Group | Where-Object { $_.HandleCount -ne "" } | ForEach-Object { [double]$_.HandleCount })
    [pscustomobject]@{
        ProcessName = $group.Name
        Samples = $group.Count
        CpuAveragePercent = if ($cpu) { [math]::Round(($cpu | Measure-Object -Average).Average, 2) } else { $null }
        CpuMaximumPercent = if ($cpu) { [math]::Round(($cpu | Measure-Object -Maximum).Maximum, 2) } else { $null }
        PrivateBytesMinimum = if ($private) { ($private | Measure-Object -Minimum).Minimum } else { $null }
        PrivateBytesMaximum = if ($private) { ($private | Measure-Object -Maximum).Maximum } else { $null }
        HandleMinimum = if ($handles) { ($handles | Measure-Object -Minimum).Minimum } else { $null }
        HandleMaximum = if ($handles) { ($handles | Measure-Object -Maximum).Maximum } else { $null }
    }
}

if ($OutputPath) {
    $resolvedOutput = [System.IO.Path]::GetFullPath($OutputPath)
    $parent = Split-Path -Parent $resolvedOutput
    if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    $summary | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $resolvedOutput -Encoding utf8NoBOM
    Write-Host "Wrote summary to $resolvedOutput"
} else {
    $summary | Format-Table -AutoSize
}
