# Derive local identity scan terms without printing or persisting their values.
# Run from any folder: powershell -NoProfile -File scripts/verify-release-local.ps1
param([switch]$CheckSource, [switch]$WithGnome, [switch]$WindowsOnly)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$previous = $env:PASTEROLL_SCAN_WORDS
$terms = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$manifest = [IO.File]::ReadAllText((Join-Path $repoRoot 'Cargo.toml'))
$publicOwner = [regex]::Match($manifest, 'repository\s*=\s*"https://github\.com/([^/]+)/').Groups[1].Value
$publicNameExcluded = $false

function Add-PrivateTerm([string]$Value) {
    if (-not [string]::IsNullOrWhiteSpace($Value)) {
        $Value = $Value.Trim()
        if ($Value -notin @('Pasteroll', 'Pasteroll contributors', 'WORKGROUP', 'localhost')) {
            [void]$terms.Add($Value)
        }
    }
}

Push-Location $repoRoot
$verificationFailed = $false
try {
    foreach ($term in ($previous -split ',')) { Add-PrivateTerm $term }
    foreach ($name in @('USERNAME', 'COMPUTERNAME', 'GIT_AUTHOR_NAME', 'GIT_COMMITTER_NAME', 'GIT_AUTHOR_EMAIL', 'GIT_COMMITTER_EMAIL', 'EMAIL')) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if ($name -in @('GIT_AUTHOR_NAME', 'GIT_COMMITTER_NAME') -and $value -eq $publicOwner) {
            $publicNameExcluded = $true
        } else { Add-PrivateTerm $value }
    }
    if ($env:USERPROFILE) {
        Add-PrivateTerm (Split-Path -Leaf $env:USERPROFILE)
        Add-PrivateTerm $env:USERPROFILE
        Add-PrivateTerm ($env:USERPROFILE.Replace('\', '/'))
    }
    $displayNameAvailable = $false
    if (Get-Command Get-LocalUser -ErrorAction SilentlyContinue) {
        $localAccount = Get-LocalUser -Name $env:USERNAME -ErrorAction SilentlyContinue
        if ($localAccount -and -not [string]::IsNullOrWhiteSpace($localAccount.FullName)) {
            $displayNameAvailable = $true
            if ($localAccount.FullName -eq $publicOwner) { $publicNameExcluded = $true }
            else { Add-PrivateTerm $localAccount.FullName }
        }
    }
    $gitEmail = $null
    foreach ($key in @('user.name', 'user.email', 'author.email', 'committer.email')) {
        $value = & git config --get $key 2>$null
        if ($LASTEXITCODE -eq 0) {
            if ($key -eq 'user.name' -and ($value -join '') -eq $publicOwner) {
                # The published project owner appears intentionally in repository/license links.
                # Keep actual account/profile and email terms; do not classify this public handle
                # as an additional private name. No arbitrary names are exempted.
                $publicNameExcluded = $true
            } else { Add-PrivateTerm ($value -join '') }
            if ($key -eq 'user.email') { $gitEmail = $value }
        }
    }
    if ($terms.Count -eq 0) { throw 'No local identity terms could be derived.' }
    # The environment is set only for this process/its children, then restored in finally.
    $env:PASTEROLL_SCAN_WORDS = $terms -join ','
    Write-Output ('Derived private scan terms: ' + $terms.Count + '; configured Git email available: ' + [bool]$gitEmail)
    Write-Output ('Local account display name checked: ' + $displayNameAvailable)
    Write-Output ('Published repository-owner name classified as public: ' + $publicNameExcluded)
    $scanArgs = @()
    if ($WithGnome) { $scanArgs += '--with-gnome' }
    if ($WindowsOnly) { $scanArgs += '--windows-only' }
    & python (Join-Path $PSScriptRoot 'verify-release.py') @scanArgs
    if ($LASTEXITCODE -ne 0) { throw 'The private-string release scan did not pass.' }
    if ($CheckSource) {
        & python (Join-Path $PSScriptRoot 'verify-source.py')
        if ($LASTEXITCODE -ne 0) { throw 'The private-string source scan did not pass.' }
    }
} catch {
    [Console]::Error.WriteLine('Local identity verification did not pass. Private terms were not displayed or saved.')
    $verificationFailed = $true
} finally {
    $env:PASTEROLL_SCAN_WORDS = $previous
    Pop-Location
}
if ($verificationFailed) { exit 1 }
