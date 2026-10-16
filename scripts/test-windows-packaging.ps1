# Inspect fresh Windows packages without installing, starting capture, or changing the registry.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'windows-package-common.ps1')
Push-Location $PackageRoot
try {
    $package = Get-PasterollPackage
    $version = $package.version
    $exe = Join-Path $PackageRoot 'target\release\pasteroll.exe'
    $setup = Join-Path $PackageRoot "dist\pasteroll-$version-windows-x64-setup.exe"
    $zip = Join-Path $PackageRoot "dist\pasteroll-$version-windows-x64-portable.zip"
    foreach ($file in @($exe, $setup)) {
        if (-not (Test-Path -LiteralPath $file)) { throw 'A required release artifact is missing.' }
        $info = [Diagnostics.FileVersionInfo]::GetVersionInfo($file)
        # Inno reserves padding in these resource strings for signing/patching.
        if ($info.FileVersion.Trim() -ne $version -or $info.CompanyName.Trim() -ne $package.authors[0]) {
            throw 'Executable version or publisher metadata does not match Cargo.'
        }
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        $expected = @('pasteroll.exe', 'README.txt', 'portable.txt', 'LICENSE-MIT', 'LICENSE-APACHE', 'THIRD-PARTY-NOTICES.md')
        if (Compare-Object ($archive.Entries.FullName | Sort-Object) ($expected | Sort-Object)) {
            throw 'Unexpected files in the portable ZIP.'
        }
        if ($archive.GetEntry('portable.txt').Length -ne 0) { throw 'portable.txt must be empty.' }
        $stream = $archive.GetEntry('pasteroll.exe').Open()
        $hash = [Security.Cryptography.SHA256]::Create()
        try {
            $payloadHash = [BitConverter]::ToString($hash.ComputeHash($stream)).Replace('-', '')
        } finally { $stream.Dispose(); $hash.Dispose() }
        if ($payloadHash -ne (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash) { throw 'ZIP executable does not match the fresh release build.' }
    } finally { $archive.Dispose() }
    $portable = Join-Path $PackageRoot 'target\windows-packaging\portable\pasteroll.exe'
    $pathOutput = & $portable config path
    if ($LASTEXITCODE -ne 0) { throw 'Portable config path command failed.' }
    $expectedPath = Join-Path (Split-Path -Parent $portable) 'data\config.toml'
    if ([IO.Path]::GetFullPath(($pathOutput -join '').Trim()) -ne [IO.Path]::GetFullPath($expectedPath)) {
        throw 'Portable configuration escaped the executable directory.'
    }
    if (Test-Path -LiteralPath (Split-Path -Parent $expectedPath)) { throw 'Config path inspection unexpectedly created data.' }
    $marker = Join-Path $PackageRoot 'target\windows-packaging\setup\setup.txt'
    if (-not (Test-Path -LiteralPath $marker) -or (Get-Item -LiteralPath $marker).Length -ne 0) { throw 'Setup ownership marker is missing or not empty.' }
    Write-Output 'PASS: Windows package contents, metadata, payload, markers and portable path.'
} finally {
    Pop-Location
}
