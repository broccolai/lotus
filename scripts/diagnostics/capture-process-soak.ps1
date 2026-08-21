[CmdletBinding()]
param(
    [int] $IntervalSeconds = 5,
    [int] $DurationMinutes = 10,
    [int] $LotusProcessId,
    [string] $LotusPath,
    [string] $OutputPath = "lotus-process-soak.csv",
    [string] $WindowsBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($IntervalSeconds -lt 1) { throw "IntervalSeconds must be at least 1." }
if ($DurationMinutes -lt 1) { throw "DurationMinutes must be at least 1." }

function Get-WindowsBuildValue {
    param([string] $Supplied)

    if ($Supplied) { return $Supplied }
    try {
        return (Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion").CurrentBuild
    } catch {
        return "unknown"
    }
}

function Get-FileMetadata {
    param([string] $Path)

    if (-not $Path -or -not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return @{ Path = $Path; Version = "unknown"; Sha256 = "unknown" }
    }

    $file = Get-Item -LiteralPath $Path
    $version = $file.VersionInfo.ProductVersion
    if (-not $version) { $version = "unknown" }
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
    return @{ Path = $file.FullName; Version = $version; Sha256 = $hash }
}

function Get-ProcessPath {
    param([System.Diagnostics.Process] $Process)

    try { return $Process.Path } catch { return $null }
}

if (-not ("Lotus.Diagnostics.NativeMethods" -as [type])) {
    Add-Type @"
using System;
using System.Runtime.InteropServices;
namespace Lotus.Diagnostics {
    public static class NativeMethods {
        [DllImport("user32.dll")]
        public static extern uint GetGuiResources(IntPtr process, uint flags);
    }
}
"@
}

function Get-ResourceCount {
    param([System.Diagnostics.Process] $Process, [uint32] $Flag)

    try {
        return [Lotus.Diagnostics.NativeMethods]::GetGuiResources($Process.Handle, $Flag)
    } catch {
        return ""
    }
}

function Get-ProcessRow {
    param(
        [System.Diagnostics.Process] $Process,
        [datetime] $SampleTime,
        [hashtable] $Previous,
        [hashtable] $Metadata,
        [double] $ElapsedSeconds
    )

    $cpu = ""
    $cpuSeconds = ""
    $workingSet = ""
    $privateBytes = ""
    $handles = ""
    $threads = ""
    $commit = ""
    $uptime = ""
    try {
        $Process.Refresh()
        $cpuTime = $Process.TotalProcessorTime.TotalSeconds
        $cpuSeconds = $cpuTime
        if ($Previous.ContainsKey($Process.Id)) {
            $delta = $cpuTime - $Previous[$Process.Id]
            $cpu = [math]::Round(($delta / [math]::Max($ElapsedSeconds, 0.001)) * 100 / [Environment]::ProcessorCount, 2)
        }
        $Previous[$Process.Id] = $cpuTime
        $workingSet = $Process.WorkingSet64
        $privateBytes = $Process.PrivateMemorySize64
        $handles = $Process.HandleCount
        $threads = $Process.Threads.Count
        $commit = $Process.PagedMemorySize64
        $uptime = [math]::Round(($SampleTime - $Process.StartTime).TotalSeconds, 3)
    } catch {
        $cpu = ""
    }

    [pscustomobject]@{
        SchemaVersion = 1
        TimestampUtc = $SampleTime.ToUniversalTime().ToString("o")
        ProcessName = $Process.ProcessName
        ProcessId = $Process.Id
        ExecutablePath = $Metadata.Path
        ProductVersion = $Metadata.Version
        ExecutableSha256 = $Metadata.Sha256
        WindowsBuild = $script:DetectedWindowsBuild
        CpuPercent = $cpu
        TotalCpuSeconds = $cpuSeconds
        WorkingSetBytes = $workingSet
        PrivateBytes = $privateBytes
        CommitBytes = $commit
        HandleCount = $handles
        ThreadCount = $threads
        GdiObjectCount = (Get-ResourceCount $Process 0)
        UserObjectCount = (Get-ResourceCount $Process 1)
        UptimeSeconds = $uptime
    }
}

$script:DetectedWindowsBuild = Get-WindowsBuildValue $WindowsBuild
$resolvedOutput = [System.IO.Path]::GetFullPath($OutputPath)
$parent = Split-Path -Parent $resolvedOutput
if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }

$lotus = if ($LotusProcessId) { Get-Process -Id $LotusProcessId -ErrorAction Stop } else { Get-Process -Name lotus -ErrorAction SilentlyContinue | Select-Object -First 1 }
$metadata = Get-FileMetadata $LotusPath
if (-not $LotusPath -and $lotus) {
    try { $metadata = Get-FileMetadata $lotus.Path } catch { }
}

$header = "# lotus-process-soak schema=1 windowsBuild=$($script:DetectedWindowsBuild) lotusVersion=$($metadata.Version) lotusSha256=$($metadata.Sha256) startedUtc=$([datetime]::UtcNow.ToString('o'))"
if (-not (Test-Path -LiteralPath $resolvedOutput)) {
    Set-Content -LiteralPath $resolvedOutput -Value $header -Encoding utf8NoBOM
    Add-Content -LiteralPath $resolvedOutput -Value "SchemaVersion,TimestampUtc,ProcessName,ProcessId,ExecutablePath,ProductVersion,ExecutableSha256,WindowsBuild,CpuPercent,TotalCpuSeconds,WorkingSetBytes,PrivateBytes,CommitBytes,HandleCount,ThreadCount,GdiObjectCount,UserObjectCount,UptimeSeconds" -Encoding utf8NoBOM
}

$previousCpu = @{}
$end = [datetime]::UtcNow.AddMinutes($DurationMinutes)
$lastSample = [datetime]::UtcNow
while ([datetime]::UtcNow -lt $end) {
    $now = [datetime]::UtcNow
    $elapsed = ($now - $lastSample).TotalSeconds
    $lastSample = $now
    $processes = @()
    if ($lotus) { $processes += $lotus }
    $processes += @(Get-Process -Name explorer -ErrorAction SilentlyContinue)
    foreach ($process in $processes | Sort-Object Id -Unique) {
        $processMetadata = if ($process.ProcessName -ieq "lotus") { $metadata } else { Get-FileMetadata (Get-ProcessPath $process) }
        Get-ProcessRow $process $now $previousCpu $processMetadata $elapsed | ConvertTo-Csv -NoTypeInformation | Select-Object -Skip 1 | Add-Content -LiteralPath $resolvedOutput -Encoding utf8NoBOM
    }
    Start-Sleep -Seconds $IntervalSeconds
}

Write-Host "Wrote process soak samples to $resolvedOutput"
