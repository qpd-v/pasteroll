# Native current-host test using only a new temporary installation and synthetic data.
# Never starts clipboard capture, enables autostart, or changes live shortcuts/history.
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$registryPath = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{E04386D3-6242-420F-864E-35305B8A20FC}_is1'
$legacyPath = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\Pasteroll'
$runPath = 'Software\Microsoft\Windows\CurrentVersion\Run'
$checks = [Collections.Generic.List[string]]::new()
$root = $null
$install = $null
$created = $false
$hosts = @()
$events = @()
$failed = $false

function Assert-Test([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Pass-Test([string]$Message) { $checks.Add($Message); Write-Output ('PASS: ' + $Message) }

function Quote-Native([string]$Value) {
    $escaped = [regex]::Replace($Value, '(\\*)"', { param($m) ('\' * ($m.Groups[1].Length * 2 + 1)) + '"' })
    $escaped = [regex]::Replace($escaped, '(\\+)$', { param($m) '\' * ($m.Length * 2) })
    return '"' + $escaped + '"'
}

function Invoke-Native([string]$File, [string[]]$Arguments, [int[]]$Allowed = @(0)) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $File
    $info.Arguments = ($Arguments | ForEach-Object { Quote-Native $_ }) -join ' '
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = [Diagnostics.Process]::Start($info)
    try {
        Assert-Test ($process.WaitForExit(30000)) 'A native test command timed out.'
        $stdout = $process.StandardOutput.ReadToEnd()
        $stderr = $process.StandardError.ReadToEnd()
        $exitCode = $process.ExitCode
        Assert-Test ($Allowed -contains $exitCode) ('A native test command failed with exit code ' + $exitCode)
        return [pscustomobject]@{ Code = $exitCode; Stdout = $stdout; Stderr = $stderr }
    } finally { $process.Dispose() }
}

function Open-UserKey([string]$Path) {
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser, [Microsoft.Win32.RegistryView]::Registry64)
    try { return $base.OpenSubKey($Path) } finally { $base.Dispose() }
}

function Key-Snapshot([string]$Path, [switch]$OnlyRunValue) {
    $key = Open-UserKey $Path
    if (-not $key) { return '<absent>' }
    try {
        $names = if ($OnlyRunValue) { @('Pasteroll') | Where-Object { $key.GetValueNames() -contains $_ } } else { $key.GetValueNames() | Sort-Object }
        $items = foreach ($name in $names) {
            [pscustomobject]@{ Name = $name; Kind = [string]$key.GetValueKind($name); Value = $key.GetValue($name) }
        }
        return ConvertTo-Json -InputObject @($items) -Compress -Depth 8
    } finally { $key.Dispose() }
}

function Link-Snapshot {
    $links = @((Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs\Pasteroll.lnk'),
               (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Pasteroll.lnk'))
    return (($links | ForEach-Object { if (Test-Path -LiteralPath $_) { (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash } else { '<absent>' } }) -join '|')
}

function Assert-LiveUnchanged {
    foreach ($entry in $live.GetEnumerator()) {
        $process = Get-Process -Id $entry.Key -ErrorAction SilentlyContinue
        Assert-Test ([bool]$process -and $process.StartTime.Ticks -eq $entry.Value) 'A live Pasteroll process was interrupted.'
    }
    Assert-Test ((Key-Snapshot $runPath -OnlyRunValue) -eq $beforeRun) 'The live autostart entry changed.'
    Assert-Test ((Key-Snapshot $legacyPath) -eq $beforeLegacy) 'The legacy installation registration changed.'
    Assert-Test ((Link-Snapshot) -eq $beforeLinks) 'A live shortcut changed.'
}

function Own-Registration {
    $key = Open-UserKey $registryPath
    if (-not $key) { return $false }
    try {
        $location = [string]$key.GetValue('InstallLocation')
        return $location.TrimEnd('\') -eq $install.TrimEnd('\')
    } finally { $key.Dispose() }
}

try {
    foreach ($hive in @([Microsoft.Win32.RegistryHive]::CurrentUser, [Microsoft.Win32.RegistryHive]::LocalMachine)) {
        $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey($hive, [Microsoft.Win32.RegistryView]::Registry64)
        try {
            $existing = $base.OpenSubKey($registryPath)
            if ($existing) { $existing.Dispose(); throw 'A Setup registration already exists; refusing to replace it.' }
        } finally { $base.Dispose() }
    }
    $live = @{}
    foreach ($process in @(Get-Process pasteroll -ErrorAction SilentlyContinue)) { $live[$process.Id] = $process.StartTime.Ticks }
    $beforeRun = Key-Snapshot $runPath -OnlyRunValue
    $beforeLegacy = Key-Snapshot $legacyPath
    $beforeLinks = Link-Snapshot
    $common = [IO.Path]::GetFullPath([Environment]::GetFolderPath('CommonApplicationData'))
    $root = [IO.Path]::GetFullPath((Join-Path $common ('Pasteroll install test ' + [Guid]::NewGuid().ToString('N'))))
    Assert-Test ([IO.Path]::GetDirectoryName($root) -eq $common) 'Unexpected temporary test path.'
    [IO.Directory]::CreateDirectory($root) | Out-Null
    $created = $true
    $install = Join-Path $root 'program'
    $data = Join-Path $root 'synthetic-data'
    $db = Join-Path $data 'history.db'
    $release = Join-Path $repoRoot 'target\release\pasteroll.exe'
    $setup = Join-Path $repoRoot 'dist\pasteroll-0.1.0-beta.3-windows-x64-setup.exe'
    Assert-Test ([Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($release)).Contains('--if-exe requires an executable path.')) 'Rebuild first: the release executable lacks scoped-stop support.'
    Assert-Test ((Get-Item -LiteralPath $setup).LastWriteTimeUtc -ge (Get-Item -LiteralPath $release).LastWriteTimeUtc) 'Rebuild Setup after rebuilding the app.'

    # Message-only native test processes prove scoped WM_CLOSE delivery, without hooks,
    # clipboard, registry or history access. Each is a different executable path.
    $source = @'
using System;
using System.Runtime.InteropServices;
using System.Threading;
public static class StopProbe {
  delegate IntPtr Callback(IntPtr h, uint m, UIntPtr w, IntPtr l);
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
  struct WC { public uint style; public Callback proc; public int ce, we; public IntPtr instance, icon, cursor, background; public string menu, name; }
  [StructLayout(LayoutKind.Sequential)]
  struct MSG { public IntPtr window; public uint message; public UIntPtr w; public IntPtr l; public uint time; public int x,y; public uint extra; }
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] static extern IntPtr GetModuleHandleW(string name);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern ushort RegisterClassW(ref WC c);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr CreateWindowExW(uint e,string c,string t,uint s,int x,int y,int w,int h,IntPtr p,IntPtr m,IntPtr i,IntPtr l);
  [DllImport("user32.dll")] static extern int GetMessageW(out MSG m,IntPtr h,uint a,uint b);
  [DllImport("user32.dll")] static extern IntPtr DispatchMessageW(ref MSG m);
  [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr h);
  [DllImport("user32.dll")] static extern void PostQuitMessage(int code);
  [DllImport("user32.dll")] static extern IntPtr DefWindowProcW(IntPtr h,uint m,UIntPtr w,IntPtr l);
  static Callback callback = Handle;
  static IntPtr Handle(IntPtr h,uint m,UIntPtr w,IntPtr l) {
    if(m==0x10) { DestroyWindow(h); return IntPtr.Zero; }
    if(m==2) { PostQuitMessage(0); return IntPtr.Zero; }
    return DefWindowProcW(h,m,w,l);
  }
  [STAThread] public static void Main(string[] args) {
    try {
      var instance=GetModuleHandleW(null);
      var c=new WC { proc=callback, instance=instance, name="PasterollApp" };
      if(RegisterClassW(ref c)==0) Environment.Exit(3);
      if(CreateWindowExW(0,"PasterollApp","synthetic stop probe",0,0,0,1,1,new IntPtr(-3),IntPtr.Zero,instance,IntPtr.Zero)==IntPtr.Zero) Environment.Exit(4);
      using(var ready=EventWaitHandle.OpenExisting(args[0])) ready.Set();
      MSG message;
      while(GetMessageW(out message,IntPtr.Zero,0,0)>0) DispatchMessageW(ref message);
    } catch { Environment.Exit(5); }
  }
}
'@
    $probeA = Join-Path $root 'probe a.exe'
    $probeB = Join-Path $root 'probe b.exe'
    $probeSource = Join-Path $root 'stop-probe.cs'
    [IO.File]::WriteAllText($probeSource, $source)
    $csc = Join-Path ([Runtime.InteropServices.RuntimeEnvironment]::GetRuntimeDirectory()) 'csc.exe'
    if (-not (Test-Path -LiteralPath $csc)) {
        $csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
    }
    Assert-Test (Test-Path -LiteralPath $csc) 'The native test requires the installed .NET Framework C# compiler.'
    Invoke-Native $csc @('/nologo', '/target:winexe', '/optimize+', '/debug-', ('/out:' + $probeA), $probeSource) | Out-Null
    Copy-Item -LiteralPath $probeA -Destination $probeB
    foreach ($probe in @($probeA, $probeB)) {
        $eventName = 'Local\PasterollStopProbe-' + [Guid]::NewGuid().ToString('N')
        $event = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $eventName)
        $events += $event
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $probe; $info.Arguments = $eventName; $info.UseShellExecute = $false; $info.CreateNoWindow = $true
        $hostProcess = [Diagnostics.Process]::Start($info)
        $hosts += $hostProcess
        Assert-Test ($event.WaitOne(5000)) 'A synthetic message-window probe did not start.'
    }
    Invoke-Native $release @('stop', '--if-exe', (Join-Path $root 'not-running.exe'), '--db', $db) | Out-Null
    Invoke-Native $release @('stop', '--db', $db, '--if-exe') @(1) | Out-Null
    Assert-Test (-not $hosts[0].HasExited -and -not $hosts[1].HasExited) 'A nonmatching stop closed a test process.'
    Invoke-Native $release @('stop', '--if-exe', $probeA, '--db', $db) | Out-Null
    Assert-Test ($hosts[0].WaitForExit(5000) -and -not $hosts[1].HasExited) 'Scoped stop did not select exactly its target.'
    Invoke-Native $release @('stop', '--if-exe', $probeA, '--db', $db) | Out-Null
    Invoke-Native $release @('stop', '--if-exe', $probeB, '--db', $db) | Out-Null
    Assert-Test ($hosts[1].WaitForExit(5000)) 'The second scoped stop did not exit gracefully.'
    Assert-LiveUnchanged
    Pass-Test 'Scoped stop selects only the requested executable and leaves live Pasteroll running'

    $setupArgs = @('/VERYSILENT', '/CURRENTUSER', '/SUPPRESSMSGBOXES', '/NORESTART', '/TASKS=', ('/DIR=' + $install))
    Invoke-Native $setup $setupArgs | Out-Null
    Assert-Test (Own-Registration) 'The current-user Setup registration was not created in the test folder.'
    $installedExe = Join-Path $install 'pasteroll.exe'
    Assert-Test ((Test-Path -LiteralPath $installedExe) -and (Test-Path -LiteralPath (Join-Path $install 'setup.txt'))) 'Installed files/ownership marker are missing.'
    Assert-LiveUnchanged
    Pass-Test 'Actual per-user silent setup installs to a temporary folder without touching the live installation'

    $message = Invoke-Native $installedExe @('install', '--db', $db)
    Assert-Test ($message.Stdout.Contains('already installed by Setup')) 'Setup ownership was not detected.'
    $message = Invoke-Native $installedExe @('uninstall', '--db', $db) @(1)
    Assert-Test ($message.Stderr.Contains('Settings > Apps')) 'Self-uninstall did not delegate to Setup.'
    Pass-Test 'Installed release recognizes Setup ownership and delegates removal'

    Invoke-Native $installedExe @('seed', '--db', $db) | Out-Null
    [IO.File]::WriteAllText((Join-Path $data 'image-presets.txt'), 'synthetic preset fixture')
    [IO.File]::WriteAllText((Join-Path $data 'unrelated.txt'), 'keep this fixture')
    $fixtureHashes = @{}
    foreach ($file in Get-ChildItem -LiteralPath $data -File) { $fixtureHashes[$file.Name] = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash }
    Invoke-Native $setup $setupArgs | Out-Null
    foreach ($name in $fixtureHashes.Keys) {
        Assert-Test ((Get-FileHash -LiteralPath (Join-Path $data $name) -Algorithm SHA256).Hash -eq $fixtureHashes[$name]) 'Reinstallation changed synthetic data.'
    }
    Assert-LiveUnchanged
    Pass-Test 'Actual in-place Setup reinstall preserves synthetic encrypted history/key/presets and live settings'

    Invoke-Native $installedExe @('uninstall', '--data-only', '--remove-data', '--yes', '--db', $db) | Out-Null
    Assert-Test ((Get-ChildItem -LiteralPath $data -File).Count -eq 1 -and (Test-Path -LiteralPath (Join-Path $data 'unrelated.txt'))) 'Data-only removal did not preserve only the unrelated fixture.'
    Assert-Test (Test-Path -LiteralPath $installedExe) 'Data-only cleanup removed the program.'
    Pass-Test 'Installed release data-only cleanup removes synthetic data and preserves unrelated files/program'
    [IO.File]::WriteAllText((Join-Path $data 'history.db'), 'synthetic retention fixture')
    Invoke-Native (Join-Path $install 'unins000.exe') @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') | Out-Null
    Assert-Test (-not (Test-Path -LiteralPath $installedExe)) 'Native uninstaller left the program behind.'
    Assert-Test (-not (Own-Registration)) 'Native uninstaller left its test registration behind.'
    Assert-Test ([IO.File]::ReadAllText((Join-Path $data 'history.db')) -eq 'synthetic retention fixture') 'Silent uninstall removed retained synthetic data.'
    Assert-LiveUnchanged
    Pass-Test 'Actual silent uninstall removes its program/registration and preserves data/live state'
} catch {
    [Console]::Error.WriteLine('FAIL: native current-host installation test: ' + $_.Exception.Message)
    $failed = $true
} finally {
    foreach ($hostProcess in $hosts) {
        if (-not $hostProcess.HasExited) {
            Invoke-Native $release @('stop', '--if-exe', $hostProcess.StartInfo.FileName, '--db', $db) | Out-Null
        }
        $hostProcess.Dispose()
    }
    foreach ($event in $events) { $event.Dispose() }
    if ($created -and $install -and (Own-Registration)) {
        $uninstaller = Join-Path $install 'unins000.exe'
        if (Test-Path -LiteralPath $uninstaller) {
            Invoke-Native $uninstaller @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') | Out-Null
        }
    }
    if ($created -and -not (Own-Registration)) {
        $resolved = [IO.Path]::GetFullPath($root)
        Assert-Test ([IO.Path]::GetDirectoryName($resolved) -eq $common -and (Split-Path -Leaf $resolved) -match '^Pasteroll install test [a-f0-9]{32}$') 'Refusing unexpected test cleanup path.'
        Assert-Test (-not ((Get-Item -LiteralPath $resolved -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) 'Refusing cleanup through a reparse point.'
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
    $report = [ordered]@{ passed = @($checks); clean_machine = $false; native_windows = $true;
        all_users_uac_tested = $false; interactive_data_prompt_tested = $false; success = (-not $failed) }
    [IO.File]::WriteAllText((Join-Path $repoRoot 'target-dev\windows-install-results.json'), ($report | ConvertTo-Json -Depth 5))
}
if ($failed) { exit 1 }
Write-Output ('PASS: ' + $checks.Count + ' native current-host installation checks; no UAC or clean-machine claim.')
