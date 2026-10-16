; Build with scripts/package-windows-installer.ps1. No components are downloaded.
; Keep this AppId unchanged for every upgrade.
#ifndef AppVersion
  #error AppVersion must come from Cargo metadata via /DAppVersion
#endif
#ifndef StageDir
  #error StageDir must name the staged release files
#endif

[Setup]
AppId={{E04386D3-6242-420F-864E-35305B8A20FC}
AppName=Pasteroll
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}
VersionInfoVersion={#AppFileVersion}
VersionInfoTextVersion={#AppVersion}
VersionInfoCompany={#AppPublisher}
VersionInfoCopyright={#AppLicense}
DefaultDirName={autopf}\Pasteroll
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
DisableWelcomePage=no
DisableProgramGroupPage=yes
DisableReadyPage=no
LicenseFile={#StageDir}\LICENSES.txt
SetupIconFile=..\..\crates\pasteroll-win\pasteroll.ico
UninstallDisplayIcon={app}\pasteroll.exe
OutputDir=..\..\dist
OutputBaseFilename=pasteroll-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=no
RestartApplications=no
SetupLogging=no
; FUTURE SIGNING HOOK: configure the pasteroll Sign Tool, then enable both lines.
; SignTool=pasteroll
; SignedUninstaller=yes
; Sign the app before staging it too. SignPath Foundation offers signing for eligible OSS projects.

[Tasks]
Name: "startmenu"; Description: "Create a Start menu shortcut"; Flags: checkedonce
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "autostart"; Description: "Start Pasteroll when I sign in (my Windows account only)"; Flags: unchecked

[Files]
Source: "{#StageDir}\pasteroll.exe"; DestDir: "{app}"; Flags: ignoreversion
; The new helper can stop older self-installed builds which do not have a stop command.
Source: "{#StageDir}\pasteroll.exe"; DestName: "pasteroll-control.exe"; Flags: dontcopy
Source: "{#StageDir}\setup.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\README.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Pasteroll"; Filename: "{app}\pasteroll.exe"; Parameters: "run"; Tasks: startmenu
Name: "{autodesktop}\Pasteroll"; Filename: "{app}\pasteroll.exe"; Parameters: "run"; Tasks: desktopicon

[Run]
Filename: "{app}\pasteroll.exe"; Parameters: "run"; Description: "Run Pasteroll"; Flags: postinstall nowait skipifsilent runasoriginaluser

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';

var
  RemoveData: Boolean;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ExitCode: Integer;
begin
  Result := '';
  if FileExists(ExpandConstant('{app}\portable.txt')) then begin
    Result := 'This folder contains a portable copy. Choose another install folder. Portable data will not be moved or removed.';
    exit;
  end;
  ExtractTemporaryFile('pasteroll-control.exe');
  if not ExecAsOriginalUser(ExpandConstant('{tmp}\pasteroll-control.exe'),
      'stop --if-exe "' + ExpandConstant('{app}\pasteroll.exe') + '"', '', SW_HIDE,
      ewWaitUntilTerminated, ExitCode) then
    Result := 'Could not request that Pasteroll quit. Quit it from the tray and try again.'
  else if ExitCode <> 0 then
    Result := 'Pasteroll did not quit. Quit all running copies from their tray icons and try again.';
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ExitCode: Integer;
begin
  if CurStep = ssPostInstall then begin
    if not ExecAsOriginalUser(ExpandConstant('{app}\pasteroll.exe'), 'install --setup-adopt', '',
        SW_HIDE, ewWaitUntilTerminated, ExitCode) then
      RaiseException('Could not synchronize the current account''s Setup registration.');
    if ExitCode <> 0 then
      RaiseException('Could not synchronize the current account''s Setup registration.');
    if WizardIsTaskSelected('autostart') then begin
      if not ExecAsOriginalUser(ExpandConstant('{app}\pasteroll.exe'), 'autostart on', '',
          SW_HIDE, ewWaitUntilTerminated, ExitCode) then
        RaiseException('Could not enable start at sign-in. You can enable it later in Settings.');
      if ExitCode <> 0 then
        RaiseException('Could not enable start at sign-in. You can enable it later in Settings.');
    end;
  end;
end;

function InitializeUninstall: Boolean;
var
  ExitCode: Integer;
begin
  Result := False;
  RemoveData := False;
  if not Exec(ExpandConstant('{app}\pasteroll.exe'),
      'stop --if-exe "' + ExpandConstant('{app}\pasteroll.exe') + '"', '', SW_HIDE,
      ewWaitUntilTerminated, ExitCode) then begin
    if not UninstallSilent then
      MsgBox('Could not request that Pasteroll quit. Quit it from the tray and try again.', mbError, MB_OK);
    exit;
  end;
  if ExitCode <> 0 then begin
    if not UninstallSilent then
      MsgBox('Pasteroll did not quit. Quit it from the tray and try again.', mbError, MB_OK);
    exit;
  end;
  if not UninstallSilent then
    RemoveData := MsgBox('Also delete my clipboard history and settings?' + #13#10#13#10 +
      'This affects only the Windows account running this uninstaller. Data for other accounts is always kept.' + #13#10 +
      'Choose No to keep all data.', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ExitCode: Integer;
  Value: String;
begin
  if CurUninstallStep = usUninstall then begin
    if RemoveData then begin
      if not Exec(ExpandConstant('{app}\pasteroll.exe'), 'uninstall --data-only --remove-data --yes', '',
          SW_HIDE, ewWaitUntilTerminated, ExitCode) then
        RaiseException('Could not remove data. The program has not been removed.');
      if ExitCode <> 0 then
        RaiseException('Could not remove data. The program has not been removed.');
    end;
    // Do not remove an autostart entry belonging to a different Pasteroll installation.
    if RegQueryStringValue(HKCU64, RunKey, 'Pasteroll', Value) then
      if CompareText(Value, '"' + ExpandConstant('{app}\pasteroll.exe') + '" run') = 0 then
        RegDeleteValue(HKCU64, RunKey, 'Pasteroll');
  end;
end;
