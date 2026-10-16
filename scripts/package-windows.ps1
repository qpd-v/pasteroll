# Build the versioned portable ZIP locally. No install or registry changes.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'windows-package-common.ps1')
Push-Location $PackageRoot
try {
    $package = Get-PasterollPackage
    Build-PasterollWindows
    $stage = New-PasterollStage 'portable'
    [IO.File]::WriteAllBytes((Join-Path $stage 'portable.txt'), [byte[]]@())
    $dist = Join-Path $PackageRoot 'dist'
    New-Item -ItemType Directory -Path $dist -Force | Out-Null
    $archive = Join-Path $dist "pasteroll-$($package.version)-windows-x64-portable.zip"
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $archive -Force
    Write-Output "Built pasteroll-$($package.version)-windows-x64-portable.zip"
} finally {
    Pop-Location
}
