[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Push-Location $repoRoot

try {
    $metadataJson = cargo metadata --format-version 1 --locked --no-deps
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed with exit code $LASTEXITCODE"
    }

    $metadata = $metadataJson | ConvertFrom-Json
}
finally {
    Pop-Location
}

$allowedLotusDependencies = [ordered]@{
    'lotus-app' = @(
        'lotus-core', 'lotus-dock', 'lotus-media', 'lotus-search', 'lotus-settings',
        'lotus-switcher', 'lotus-ui', 'lotus-windows'
    )
    'lotus-core' = @()
    'lotus-dock' = @('lotus-core', 'lotus-media', 'lotus-ui')
    'lotus-explorer-bridge' = @()
    'lotus-media' = @('lotus-ui')
    # Standalone native scene-capture executable; never a production feature dependency.
    'lotus-photo' = @('lotus-core', 'lotus-dock', 'lotus-search', 'lotus-switcher', 'lotus-ui', 'lotus-windows')
    'lotus-search' = @('lotus-core', 'lotus-ui')
    'lotus-settings' = @('lotus-core', 'lotus-ui')
    'lotus-shell-bridge' = @()
    'lotus-switcher' = @('lotus-core', 'lotus-ui')
    'lotus-ui' = @()
    'lotus-update' = @('lotus-core')
    # Transitional adapter edges: lotus-windows currently hosts renderer and native adapters.
    'lotus-windows' = @(
        'lotus-core', 'lotus-dock', 'lotus-media', 'lotus-shell-bridge',
        'lotus-switcher', 'lotus-ui', 'lotus-update'
    )
}

$platformPackagePattern = '^(windows($|[-_])|minhook$|winresource$)'
$platformPackageOwners = @(
    'lotus-explorer-bridge', 'lotus-shell-bridge', 'lotus-windows'
)
$workspacePackageIds = @($metadata.workspace_members)
$workspacePackages = @($metadata.packages | Where-Object { $workspacePackageIds -contains $_.id })
$workspaceNames = @($workspacePackages.name | Sort-Object -Unique)
$violations = [System.Collections.Generic.List[string]]::new()

$unmappedPackages = @($workspaceNames | Where-Object { -not $allowedLotusDependencies.Contains($_) })
foreach ($package in $unmappedPackages) {
    $violations.Add("workspace package '$package' is missing from the allowed dependency matrix")
}

$staleMatrixEntries = @($allowedLotusDependencies.Keys | Where-Object { $_ -notin $workspaceNames })
foreach ($package in $staleMatrixEntries) {
    $violations.Add("allowed dependency matrix names missing workspace package '$package'")
}

foreach ($package in $workspacePackages) {
    $actualLotusDependencies = @(
        $package.dependencies |
            Where-Object { $_.name -in $workspaceNames } |
            ForEach-Object name |
            Sort-Object -Unique
    )
    $expectedLotusDependencies = @($allowedLotusDependencies[$package.name] | Sort-Object -Unique)

    $unexpectedDependencies = @($actualLotusDependencies | Where-Object { $_ -notin $expectedLotusDependencies })
    foreach ($dependency in $unexpectedDependencies) {
        $violations.Add("$($package.name) -> $dependency is not an allowed Lotus dependency")
    }

    $missingDependencies = @($expectedLotusDependencies | Where-Object { $_ -notin $actualLotusDependencies })
    foreach ($dependency in $missingDependencies) {
        $violations.Add("$($package.name) must declare documented Lotus dependency $dependency, or update this matrix deliberately")
    }

    foreach ($dependency in $package.dependencies) {
        if ($dependency.req -eq '*' -and $null -eq $dependency.path) {
            $violations.Add("$($package.name) uses an external wildcard requirement for $($dependency.name)")
        }

        $isAppBuildResource = $package.name -eq 'lotus-app' -and
            $dependency.name -eq 'winresource' -and
            $dependency.kind -eq 'build'
        if ($dependency.name -match $platformPackagePattern -and
            -not $isAppBuildResource -and
            $package.name -notin $platformPackageOwners) {
            $violations.Add("$($package.name) directly depends on platform crate $($dependency.name); only $($platformPackageOwners -join ', ') may do so")
        }
    }
}

if ($violations.Count -gt 0) {
    Write-Error "Lotus architecture boundary check failed:`n - $($violations -join "`n - ")"
    exit 1
}

Write-Host "Lotus architecture boundary check passed for $($workspaceNames.Count) workspace crates."
