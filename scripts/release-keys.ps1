<#
.SYNOPSIS
  Release-signing keys for Pasteroll's "Download and install update" (ECDSA P-256, SHA-256).

.DESCRIPTION
  generate  Create a key pair. The private key is encrypted with a passphrase you type (twice).
            Writes <Name>.key.enc (private, encrypted) and <Name>.pub (public, 64 bytes, base64).
  sign      Sign a file with an encrypted private key (asks for the passphrase) and check the result
            against the matching public key. The signature is 64 bytes (r then s), written as base64.

  Pasteroll checks the signature with Windows' own crypto, so only the public keys are in the program.
  Keep the private keys out of the repository and out of any synced folder.

  Run it in its own window so it can ask for the passphrase without anything else seeing it:
    start pwsh -NoProfile -File scripts\release-keys.ps1 generate -Name primary -OutDir <folder>
#>
param(
    [Parameter(Mandatory, Position = 0)][ValidateSet('generate', 'sign')][string]$Command,
    [string]$Name,
    [string]$OutDir,
    [string]$KeyFile,
    [string]$PubFile,
    [string]$InFile,
    [string]$OutFile,
    # Tests only: take the passphrase from PASTEROLL_KEYS_TEST_PASSPHRASE. Refused outside the temp folder.
    [switch]$Test,
    # sign: do not ask for the "yes" that confirms what is being signed
    [switch]$Yes
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security
$crypto = 'System.Security.Cryptography'
$sha256 = [Security.Cryptography.HashAlgorithmName]::SHA256
$p1363 = [Security.Cryptography.DSASignatureFormat]::IeeeP1363FixedFieldConcatenation
$pbe = [Security.Cryptography.PbeParameters]::new([Security.Cryptography.PbeEncryptionAlgorithm]::Aes256Cbc, $sha256, 600000)

function Read-Passphrase([string]$prompt, [bool]$confirm) {
    if ($Test) {
        $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
        $where = [IO.Path]::GetFullPath($(if ($OutDir) { $OutDir } else { Split-Path -Parent $KeyFile }))
        if (-not $where.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase)) { throw '-Test is only for the temp folder.' }
        return $env:PASTEROLL_KEYS_TEST_PASSPHRASE
    }
    $read = {
        param($text)
        $secure = Read-Host -Prompt $text -AsSecureString
        $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
        try { [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr) } finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) }
    }
    $first = & $read $prompt
    if ($confirm) {
        # losing control of a release key cannot be undone, and the key file can be attacked offline
        if ($first.Length -lt 20) { throw 'Use at least 20 characters: five or six random words, or a long random string.' }
        if ($first -ne (& $read 'Type it again')) { throw 'The two passphrases differ. Nothing was written.' }
    }
    $first
}

function Fingerprint([byte[]]$pub) {
    $hash = [Security.Cryptography.SHA256]::HashData($pub)
    -join ($hash[0..7] | ForEach-Object { $_.ToString('x2') })
}

function Load-Key([string]$file, [string]$passphrase) {
    $encrypted = [Convert]::FromBase64String((Get-Content -Raw -LiteralPath $file).Trim())
    $key = [Security.Cryptography.ECDsa]::Create()
    $read = 0
    try { $key.ImportEncryptedPkcs8PrivateKey([Text.Encoding]::UTF8.GetBytes($passphrase), $encrypted, [ref]$read) }
    catch { throw 'Wrong passphrase, or this is not a key file made by this script.' }
    $key
}

function Public-Bytes($key) {
    $q = $key.ExportParameters($false).Q
    [byte[]]($q.X + $q.Y)
}

# a -Test run outside the temp folder is refused before anything is created
if ($Test) { [void](Read-Passphrase 'check' $false) }
switch ($Command) {
    'generate' {
        if (-not $Name -or $Name -notmatch '^[a-z0-9-]{1,30}$') { throw '-Name is required (lower-case letters, digits, dashes).' }
        if (-not $OutDir) { throw '-OutDir is required.' }
        # never inside the repository or a cloud-synced folder
        $repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
        $target = [IO.Path]::GetFullPath($OutDir)
        if ($target.StartsWith($repoRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Keep keys out of the repository.' }
        if ($target -match 'OneDrive|Dropbox|Google Drive|iCloudDrive') { throw 'Keep keys out of cloud-synced folders.' }
        New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
        $keyPath = Join-Path $OutDir "$Name.key.enc"
        $pubPath = Join-Path $OutDir "$Name.pub"
        if ((Test-Path -LiteralPath $keyPath) -or (Test-Path -LiteralPath $pubPath)) { throw "Refusing to overwrite $Name in $OutDir." }
        $passphrase = Read-Passphrase "Passphrase for the '$Name' key" $true
        $key = [Security.Cryptography.ECDsa]::Create([Security.Cryptography.ECCurve]::CreateFromFriendlyName('nistP256'))
        $pub = Public-Bytes $key
        $encrypted = $key.ExportEncryptedPkcs8PrivateKey([Text.Encoding]::UTF8.GetBytes($passphrase), $pbe)
        # prove the file works before relying on it: decrypt it, sign, verify against the public key
        $tmp = "$keyPath.tmp"
        Set-Content -LiteralPath $tmp -Value ([Convert]::ToBase64String($encrypted)) -Encoding ascii
        try {
            $again = Load-Key $tmp $passphrase
            $probe = [Text.Encoding]::UTF8.GetBytes('pasteroll release key self-test')
            $sig = $again.SignData($probe, $sha256, $p1363)
            if (-not $key.VerifyData($probe, $sig, $sha256, $p1363)) { throw 'Self-test failed.' }
            Move-Item -LiteralPath $tmp -Destination $keyPath
        } finally { Remove-Item -LiteralPath $tmp -ErrorAction SilentlyContinue }
        Set-Content -LiteralPath $pubPath -Value ([Convert]::ToBase64String($pub)) -Encoding ascii
        Write-Host "Created $keyPath"
        Write-Host "Created $pubPath"
        Write-Host "Public key fingerprint: $(Fingerprint $pub)"
        Write-Host 'Self-test passed: the encrypted file decrypts with your passphrase and signs.'
        if (-not $Test) { Read-Host 'Press Enter to close this window' | Out-Null }
    }
    'sign' {
        foreach ($required in 'KeyFile', 'InFile', 'OutFile') { if (-not (Get-Variable $required -ValueOnly)) { throw "-$required is required." } }
        if (-not $PubFile) { $PubFile = $KeyFile -replace '\.key\.enc$', '.pub' }
        # show what is about to be signed, and sign only a manifest in the exact expected form
        $text = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($InFile))
        $shape = '^pasteroll-update-manifest 1\nversion (\S+)\nsetup (\S+) (\d+) ([0-9a-f]{64})\nexe (\S+) (\d+) ([0-9a-f]{64})\n$'
        $m = [regex]::Match($text, $shape)
        if (-not $m.Success) { throw 'That file is not a Pasteroll update manifest (see scripts/make-update-manifest.py). Nothing was signed.' }
        Write-Host "About to sign a release of Pasteroll $($m.Groups[1].Value):"
        Write-Host "  installer  $($m.Groups[2].Value)  $($m.Groups[3].Value) bytes  sha256 $($m.Groups[4].Value.Substring(0, 16))..."
        Write-Host "  program    $($m.Groups[5].Value)  $($m.Groups[6].Value) bytes  sha256 $($m.Groups[7].Value.Substring(0, 16))..."
        if (-not $Test -and -not $Yes) {
            if ((Read-Host 'Type yes to sign it') -ne 'yes') { throw 'Not signed.' }
        }
        $passphrase = Read-Passphrase "Passphrase for $(Split-Path -Leaf $KeyFile)" $false
        $key = Load-Key $KeyFile $passphrase
        $pub = [Convert]::FromBase64String((Get-Content -Raw -LiteralPath $PubFile).Trim())
        if (-not [Linq.Enumerable]::SequenceEqual([byte[]](Public-Bytes $key), [byte[]]$pub)) { throw 'The key does not match its .pub file.' }
        $data = [IO.File]::ReadAllBytes($InFile)
        $sig = $key.SignData($data, $sha256, $p1363)
        if (-not $key.VerifyData($data, $sig, $sha256, $p1363)) { throw 'The new signature does not verify.' }
        Set-Content -LiteralPath $OutFile -Value ([Convert]::ToBase64String($sig)) -Encoding ascii
        Write-Host "Signed $InFile -> $OutFile (key $(Fingerprint $pub))"
    }
}
