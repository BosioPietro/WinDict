; The WinDict installer: what WinGet installs, and what "Update app" runs.
;
; It installs to Program Files, the only kind of place where Windows lets an
; app have UI access, and puts two builds there: WinDict.exe, which every
; shortcut and the Run entry start, and WinDict-uiaccess.exe, which can show
; the card above Start and Search once uiaccess.ps1 has signed it. When UI
; access is on, WinDict.exe hands over to it (see src/installed.rs).
;
; An update replaces both builds, so after every install the signing is done
; again for those who turned UI access on. update.ps1 is the scheduled task's
; script that downloads and runs the latest setup. wd.exe is WinDict on the
; command line, and the folder is put on the PATH for it.
;
; Build with the three executables in ..\dist:
;     iscc /DVersion=0.26.0 installer\windict.iss

#ifndef Version
  #error Pass the version: iscc /DVersion=x.y.z
#endif
#ifndef Dist
  #define Dist "..\dist"
#endif

#define EnvironmentKey "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"

[Setup]
; Never change it: it's how Windows and WinGet know an install is WinDict's.
AppId={{DDA83B7B-331F-48D2-BD93-4B1945B33E32}
AppName=WinDict
AppVersion={#Version}
AppVerName=WinDict {#Version}
AppPublisher=Bosio Pietro
AppPublisherURL=https://windict.bosio.zip
AppSupportURL=https://github.com/BosioPietro/WinDict/issues
AppUpdatesURL=https://github.com/BosioPietro/WinDict/releases
VersionInfoVersion={#Version}
DefaultDirName={autopf}\WinDict
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; It closes WinDict itself, and starts it again if it was running.
CloseApplications=no
OutputDir={#Dist}
; Not published as it is: the setup people download, WinGet and "Update app"
; run carries it (src/setup.rs), as WinDict-v<version>-setup.exe.
OutputBaseFilename=WinDict-v{#Version}-inno
SetupIconFile=..\assets\windict.ico
UninstallDisplayIcon={app}\WinDict.exe
LicenseFile=..\LICENSE
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
; The PATH, for wd.
ChangesEnvironment=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "it"; MessagesFile: "compiler:Languages\Italian.isl"
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"

[Files]
Source: "{#Dist}\WinDict.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Dist}\WinDict-uiaccess.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Dist}\wd.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "uiaccess.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "update.ps1"; DestDir: "{app}"; Flags: ignoreversion

[UninstallDelete]
Type: filesandordirs; Name: "{app}\.download-*"
; Inno only removes folders it created itself.
Type: dirifempty; Name: "{app}"

[Registry]
; How WinDict tells it's the installed copy. uiaccess.ps1 adds UIAccess here.
Root: HKLM; Subkey: "Software\WinDict"; ValueType: string; ValueName: "InstallDir"; ValueData: "{app}"; Flags: uninsdeletekey
; So wd can be run from any folder. Uninstalling takes it off (RemovePath).
Root: HKLM; Subkey: "{#EnvironmentKey}"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Check: NeedsAddPath

[Icons]
; The ID WinDict gives itself (src/jumplist.rs), so its Jump List's tasks
; show under this shortcut in Start.
Name: "{autoprograms}\WinDict"; Filename: "{app}\WinDict.exe"; AppUserModelID: "BosioPietro.WinDict"

[Run]
; In order, before WinDict starts again: the new WinDict-uiaccess.exe isn't
; signed yet. If signing fails, WinDict.exe runs without UI access instead.
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\uiaccess.ps1"" -Resign"; Flags: runhidden
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\update.ps1"" -Register"; Flags: runhidden
; Through Explorer, so WinDict runs as the user, not elevated like Setup.
Filename: "{win}\explorer.exe"; Parameters: """{app}\WinDict.exe"""; Description: "{cm:LaunchProgram,WinDict}"; Flags: postinstall nowait skipifsilent
Filename: "{win}\explorer.exe"; Parameters: """{app}\WinDict.exe"""; Flags: nowait skipifnotsilent; Check: WasRunning

[Code]
var
  Running: Boolean;

function WasRunning: Boolean;
begin
  Result := Running;
end;

{ Runs one of the scripts installed beside WinDict. }
function RunScript(const Script, Params: String): Boolean;
var
  Code: Integer;
begin
  Result := Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' +
      ExpandConstant('{app}\') + Script + '" ' + Params,
    '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0);
  if not Result then
    Log(Script + ' ' + Params + ' failed');
end;

function Stop(const Image: String): Boolean;
var
  Code: Integer;
begin
  { taskkill exits with 0 when it stopped something, 128 when nothing ran. }
  Result := Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM ' + Image,
    '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0);
end;

{ Stops WinDict, so its files can be replaced; returns whether it ran. }
function StopWinDict: Boolean;
begin
  Result := Stop('WinDict.exe');
  Result := Stop('WinDict-uiaccess.exe') or Result;
  if Result then
    Sleep(300);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Running := StopWinDict;
  Result := '';
end;

{ Whether the folder WinDict is in isn't on the PATH yet. }
function NeedsAddPath: Boolean;
var
  Paths: String;
begin
  if RegQueryStringValue(HKLM, '{#EnvironmentKey}', 'Path', Paths) then
    Result := Pos(';' + Lowercase(ExpandConstant('{app}')) + ';',
      ';' + Lowercase(Paths) + ';') = 0
  else
    Result := True;
end;

{ Takes the folder WinDict is in off the PATH again. }
procedure RemovePath;
var
  Paths, Dir: String;
  At: Integer;
begin
  if not RegQueryStringValue(HKLM, '{#EnvironmentKey}', 'Path', Paths) then
    Exit;
  { Between semicolons, so it's never part of another folder's name. }
  Dir := ';' + Lowercase(ExpandConstant('{app}')) + ';';
  Paths := ';' + Paths + ';';
  At := Pos(Dir, Lowercase(Paths));
  if At = 0 then
    Exit;
  Delete(Paths, At, Length(Dir) - 1);
  Paths := Copy(Paths, 2, Length(Paths) - 2);
  RegWriteExpandStringValue(HKLM, '{#EnvironmentKey}', 'Path', Paths);
end;

{ Removes "Start with Windows", if it starts this copy. }
procedure RemoveAutostart;
var
  Command: String;
begin
  if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run',
      'WinDict', Command) and
    (Pos(Lowercase(ExpandConstant('{app}\')), Lowercase(Command)) > 0) then
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'WinDict');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    StopWinDict;
    RunScript('uiaccess.ps1', '-Disable');
    RunScript('update.ps1', '-Unregister');
    RemoveAutostart;
    RemovePath;
  end;
end;
