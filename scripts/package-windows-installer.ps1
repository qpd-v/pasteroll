# Build the setup EXE and portable ZIP. Requires an existing Inno Setup 6 installation.
param([string]$Iscc)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'windows-package-common.ps1')
if (-not $Iscc) {
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) { $Iscc = $command.Source }
    foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles, (Join-Path $env:LOCALAPPDATA 'Programs'))) {
        if (-not $Iscc -and $base) {
            $candidate = Join-Path $base 'Inno Setup 6\ISCC.exe'
            if (Test-Path -LiteralPath $candidate) { $Iscc = $candidate }
        }
    }
}
if (-not $Iscc -or -not (Test-Path -LiteralPath $Iscc)) { throw 'ISCC is missing. Install Inno Setup 6; this script does not download tools.' }
# ISCC's file-version resource is 0.0.0.0; check its own banner instead.
$compilerInfo = [Diagnostics.ProcessStartInfo]::new()
$compilerInfo.FileName = $Iscc
$compilerInfo.Arguments = '/?'
$compilerInfo.UseShellExecute = $false
$compilerInfo.CreateNoWindow = $true
$compilerInfo.RedirectStandardOutput = $true
$compilerInfo.RedirectStandardError = $true
$compilerProbe = [Diagnostics.Process]::Start($compilerInfo)
$compilerBanner = $compilerProbe.StandardOutput.ReadToEnd() + $compilerProbe.StandardError.ReadToEnd()
$compilerProbe.WaitForExit()
$compilerProbe.Dispose()
if ($compilerBanner -notmatch 'Inno Setup 6 Command-Line Compiler') { throw 'This script requires Inno Setup 6.' }
Push-Location $PackageRoot
try {
    $package = Get-PasterollPackage
    & (Join-Path $PSScriptRoot 'package-windows.ps1')
    $stage = New-PasterollStage 'setup'
    [IO.File]::WriteAllBytes((Join-Path $stage 'setup.txt'), [byte[]]@())
    $licenses = "Pasteroll is dual-licensed under $($package.license). Choose either license.`r`n`r`n" +
        [IO.File]::ReadAllText((Join-Path $PackageRoot 'LICENSE-MIT')) + "`r`n`r`n" +
        [IO.File]::ReadAllText((Join-Path $PackageRoot 'LICENSE-APACHE'))
    [IO.File]::WriteAllText((Join-Path $stage 'LICENSES.txt'), $licenses, [Text.UTF8Encoding]::new($false))
    $versionNumbers = ($package.version -split '-')[0] + '.0'
    $defines = @('/Qp', "/DAppVersion=$($package.version)", "/DAppFileVersion=$versionNumbers",
        "/DAppPublisher=$($package.authors[0])", "/DAppURL=$($package.homepage)",
        "/DAppLicense=$($package.license)", "/DStageDir=$stage")
    & $Iscc @defines (Join-Path $PackageRoot 'packaging\windows\pasteroll.iss')
    if ($LASTEXITCODE -ne 0) { throw 'ISCC failed.' }
    Write-Output "Built pasteroll-$($package.version)-windows-x64-setup.exe"
} finally {
    Pop-Location
}
