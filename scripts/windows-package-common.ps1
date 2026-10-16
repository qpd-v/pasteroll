# Shared local release build and staging. Dot-source from a packaging entry point.
$ErrorActionPreference = 'Stop'
$PackageRoot = Split-Path -Parent $PSScriptRoot

function Get-PasterollPackage {
    $json = & cargo metadata --format-version 1 --no-deps --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed.' }
    $metadata = ($json -join "`n") | ConvertFrom-Json
    $package = $metadata.packages | Where-Object name -eq 'pasteroll-win'
    if (-not $package -or $package.version -notmatch '^\d+\.\d+\.\d+(-[A-Za-z0-9.-]+)?$') {
        throw 'Cannot read the Windows package version.'
    }
    return $package
}

function Build-PasterollWindows {
    $previousFlags = $env:CARGO_ENCODED_RUSTFLAGS
    try {
        # Encoded arguments preserve spaces in account names and checkout paths.
        $env:CARGO_ENCODED_RUSTFLAGS = @("--remap-path-prefix=$env:USERPROFILE=C:\user",
            "--remap-path-prefix=$PackageRoot=C:\src", '-C', 'link-arg=/PDBALTPATH:pasteroll.pdb') -join [char]31
        & cargo build --release --locked --offline --target-dir (Join-Path $PackageRoot 'target') -p pasteroll-win
        if ($LASTEXITCODE -ne 0) { throw 'Windows release build failed.' }
    } finally {
        $env:CARGO_ENCODED_RUSTFLAGS = $previousFlags
    }
}

function New-PasterollStage([string]$Name) {
    # Only reset a direct child of our dedicated build-output folder.
    if ($Name -notmatch '^[a-z0-9-]+$') { throw 'Invalid staging folder name.' }
    $base = [IO.Path]::GetFullPath((Join-Path $PackageRoot 'target\windows-packaging'))
    $stage = [IO.Path]::GetFullPath((Join-Path $base $Name))
    if ([IO.Path]::GetDirectoryName($stage) -ne $base) { throw 'Staging path escaped the build folder.' }
    foreach ($check in @((Join-Path $PackageRoot 'target'), $base, $stage)) {
        if ((Test-Path -LiteralPath $check) -and ((Get-Item -LiteralPath $check -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'Refusing to reset staging through a junction or symbolic link.'
        }
    }
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $PackageRoot 'target\release\pasteroll.exe') -Destination $stage
    Copy-Item -LiteralPath (Join-Path $PackageRoot 'packaging\README-windows.txt') -Destination (Join-Path $stage 'README.txt')
    foreach ($name in @('LICENSE-MIT', 'LICENSE-APACHE', 'THIRD-PARTY-NOTICES.md')) {
        Copy-Item -LiteralPath (Join-Path $PackageRoot $name) -Destination $stage
    }
    return $stage
}
