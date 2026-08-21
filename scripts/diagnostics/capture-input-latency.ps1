[CmdletBinding()]
param(
    [int] $IntervalSeconds = 1,
    [int] $DurationMinutes = 10,
    [int] $LotusProcessId,
    [string] $InputEventsPath,
    [string] $OutputPath = "lotus-input-latency.csv",
    [string] $WindowsBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($IntervalSeconds -lt 1) { throw "IntervalSeconds must be at least 1." }
if ($DurationMinutes -lt 1) { throw "DurationMinutes must be at least 1." }

function Get-WindowsBuildValue {
    param([string] $Supplied)
    if ($Supplied) { return $Supplied }
    try { return (Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion").CurrentBuild }
    catch { return "unknown" }
}

$build = Get-WindowsBuildValue $WindowsBuild
$resolvedOutput = [System.IO.Path]::GetFullPath($OutputPath)
$parent = Split-Path -Parent $resolvedOutput
if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
$header = "# lotus-input-latency schema=1 windowsBuild=$build captureMode=process-and-event-observer startedUtc=$([datetime]::UtcNow.ToString('o'))"
if (-not (Test-Path -LiteralPath $resolvedOutput)) {
    Set-Content -LiteralPath $resolvedOutput -Value $header -Encoding utf8NoBOM
    Add-Content -LiteralPath $resolvedOutput -Value "SchemaVersion,TimestampUtc,LotusProcessId,LotusCpuPercent,LotusPrivateBytes,LotusThreadCount,LotusHandleCount,InputEventType,InputEventTimestampUtc,InputEventPayload" -Encoding utf8NoBOM
}

$lotus = if ($LotusProcessId) { Get-Process -Id $LotusProcessId -ErrorAction Stop } else { Get-Process -Name lotus -ErrorAction SilentlyContinue | Select-Object -First 1 }
$previousCpu = $null
$lastSample = [datetime]::UtcNow
$eventOffset = 0L
$end = [datetime]::UtcNow.AddMinutes($DurationMinutes)
while ([datetime]::UtcNow -lt $end) {
    $now = [datetime]::UtcNow
    $elapsed = [math]::Max(($now - $lastSample).TotalSeconds, 0.001)
    $lastSample = $now
    $cpu = ""
    $privateBytes = ""
    $threads = ""
    $handles = ""
    if ($lotus) {
        try {
            $lotus.Refresh()
            $currentCpu = $lotus.TotalProcessorTime.TotalSeconds
            if ($null -ne $previousCpu) { $cpu = [math]::Round((($currentCpu - $previousCpu) / $elapsed) * 100 / [Environment]::ProcessorCount, 2) }
            $previousCpu = $currentCpu
            $privateBytes = $lotus.PrivateMemorySize64
            $threads = $lotus.Threads.Count
            $handles = $lotus.HandleCount
        } catch { }
    }

    $events = @()
    if ($InputEventsPath -and (Test-Path -LiteralPath $InputEventsPath -PathType Leaf)) {
        $lines = Get-Content -LiteralPath $InputEventsPath
        if ($eventOffset -lt $lines.Count) { $events = $lines[$eventOffset..($lines.Count - 1)] }
        $eventOffset = $lines.Count
    }
    if (-not $events) { $events = @($null) }
    foreach ($event in $events) {
        $eventType = "sample"
        $eventTimestamp = ""
        $eventPayload = ""
        if ($event) {
            $eventType = "diagnostic-event"
            $eventTimestamp = $now.ToString("o")
            $eventPayload = $event
        }
        [pscustomobject]@{
            SchemaVersion = 1
            TimestampUtc = $now.ToUniversalTime().ToString("o")
            LotusProcessId = if ($lotus) { $lotus.Id } else { "" }
            LotusCpuPercent = $cpu
            LotusPrivateBytes = $privateBytes
            LotusThreadCount = $threads
            LotusHandleCount = $handles
            InputEventType = $eventType
            InputEventTimestampUtc = $eventTimestamp
            InputEventPayload = $eventPayload
        } | ConvertTo-Csv -NoTypeInformation | Select-Object -Skip 1 | Add-Content -LiteralPath $resolvedOutput -Encoding utf8NoBOM
    }
    Start-Sleep -Seconds $IntervalSeconds
}

Write-Host "Wrote input latency observations to $resolvedOutput"
