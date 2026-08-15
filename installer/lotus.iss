#define MyAppName "lotus"
#define MyAppPublisher "lotus contributors"
#define MyAppExeName "lotus.exe"
#define MyAppVersion GetEnv("LOTUS_VERSION")

[Setup]
AppId={{EB208C8B-11C0-4B22-93A9-8113140647AA}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={localappdata}\Programs\Lotus
DefaultGroupName=lotus
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\crates\lotus-app\assets\lotus.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
CloseApplications=yes
RestartApplications=no
AppMutex=Local\Lotus.Dock.SingleInstance
OutputDir=..\dist
OutputBaseFilename=lotus-v{#MyAppVersion}-windows-x86_64-setup
VersionInfoVersion={#MyAppVersion}
VersionInfoProductName={#MyAppName}
VersionInfoProductVersion={#MyAppVersion}

[Files]
Source: "..\target\release\lotus.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\lotus_shell_bridge.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\lotus_explorer_bridge.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\lotus"; Filename: "{app}\{#MyAppExeName}"

[Run]
Filename: "{app}\{#MyAppExeName}"; Parameters: "{code:UpdateLaunchParameters}"; Flags: nowait; Check: RestartAfterUpdate
Filename: "{app}\{#MyAppExeName}"; Description: "Launch lotus"; Flags: nowait postinstall skipifsilent; Check: not RestartAfterUpdate

[Code]
var
  LegacyPortableInstall: Boolean;

function GetCurrentProcessId: LongWord;
  external 'GetCurrentProcessId@kernel32.dll stdcall';

function InitializeSetup: Boolean;
begin
  LegacyPortableInstall :=
    FileExists(ExpandConstant('{localappdata}\Programs\Lotus\lotus.exe')) and
    not FileExists(ExpandConstant('{localappdata}\Programs\Lotus\unins000.exe')) and
    not RegKeyExists(
      HKCU,
      'Software\Microsoft\Windows\CurrentVersion\Uninstall\{EB208C8B-11C0-4B22-93A9-8113140647AA}_is1'
    );
  Result := True;
end;

function RestartAfterUpdate: Boolean;
begin
  Result := ExpandConstant('{param:RESTARTLOTUS|0}') = '1';
end;

function UpdateLaunchParameters(Param: String): String;
begin
  Result := '--restart-after ' + IntToStr(GetCurrentProcessId) +
    ' --cleanup-update "' + ExpandConstant('{src}') + '" --open-settings';
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and LegacyPortableInstall then
    DeleteFile(ExpandConstant('{localappdata}\Lotus\settings.json'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Lotus');
end;
