; Inno Setup script for Sky Island. Build it with scripts\package.ps1, or open it in the
; Inno Setup compiler after running `cargo build --release`.

#ifndef AppVersion
  #define AppVersion "1.4.3"
#endif
#define AppName "Sky Island"
#define AppExe "sky-island.exe"

[Setup]
; Keep this ID the same forever: it is how Windows recognises a new version as an upgrade.
AppId={{986D1ACF-1D01-49B4-9F36-3906A88FAA7B}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Dannyz15
AppPublisherURL=https://github.com/Dannyz15/Sky-Island
AppSupportURL=https://github.com/Dannyz15/Sky-Island
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
; Installs for the current user only, so no administrator prompt is needed.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
LicenseFile=..\LICENSE
SetupIconFile=..\assets\icon.ico
; The welcome page is off by default in Inno Setup 6; turn it on so the big picture is shown.
DisableWelcomePage=no
WizardImageFile=wizard-large.png
WizardSmallImageFile=wizard-small.png
UninstallDisplayIcon={app}\{#AppExe}
OutputDir=..\dist
OutputBaseFilename=Sky-Island-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked
Name: "autostart"; Description: "Start {#AppName} when I sign in to Windows"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD-PARTY-LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; The same per-user startup entry the app's own "Start with Windows" setting uses.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Sky Island"; ValueData: """{app}\{#AppExe}"""; Tasks: autostart

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent

[Code]
{ True while a copy of the app is running (find answers 0 when tasklist lists it). }
function AppIsRunning: Boolean;
var
  ResultCode: Integer;
begin
  Result := Exec(ExpandConstant('{cmd}'), '/C tasklist /FI "IMAGENAME eq {#AppExe}" /NH | find /I "{#AppExe}"',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode) and (ResultCode = 0);
end;

{ A running copy would keep the exe locked, so close it before installing or removing. It is asked to
  close first, so it can finish a save, and is only forced if it is still there a few seconds later. }
procedure StopRunningApp;
var
  ResultCode: Integer;
  Waited: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM {#AppExe}', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Waited := 0;
  while AppIsRunning and (Waited < 4000) do
  begin
    Sleep(250);
    Waited := Waited + 250;
  end;
  if AppIsRunning then
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM {#AppExe} /F', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopRunningApp;
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    StopRunningApp;
    { Remove the startup entry whether the installer or the app's own setting created it. }
    RegDeleteValue(HKEY_CURRENT_USER, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Sky Island');
  end
  else if CurUninstallStep = usPostUninstall then
  begin
    { Ask before deleting the user's data. The default answer is No, and a silent
      uninstall never deletes it. }
    DataDir := ExpandConstant('{userappdata}\Sky-Island');
    if DirExists(DataDir) and (not UninstallSilent) then
    begin
      if MsgBox('Do you also want to delete your Sky Island data?' + #13#10 + #13#10 +
                'This removes your settings, to-do list, shelf, water log and saved lyrics. ' +
                'It cannot be undone.', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(DataDir, True, True, True);
    end;
  end;
end;
