; Per-user installer for rcord: installs in %LOCALAPPDATA%\Programs\rcord with Start menu and
; desktop shortcuts and no admin prompt. Built by the release workflow with
;   iscc /DVersion=0.1.0 /DSource=<folder with rcord.exe> packaging\windows\rcord.iss

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef Source
  #define Source "..\..\target\release"
#endif

[Setup]
AppId={{6B0E5C1A-3E2F-4D7B-9A51-8C2D4F6E1B37}
AppName=rcord
AppVersion={#Version}
AppVerName=rcord {#Version}
AppPublisher=khaodius
AppPublisherURL=https://github.com/khaodius/rcord
AppSupportURL=https://github.com/khaodius/rcord/issues
AppUpdatesURL=https://github.com/khaodius/rcord/releases
DefaultDirName={localappdata}\Programs\rcord
DefaultGroupName=rcord
DisableProgramGroupPage=yes
DisableDirPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\dist
OutputBaseFilename=rcord-{#Version}-x64-setup
SetupIconFile=..\..\assets\icons\rcord.ico
UninstallDisplayIcon={app}\rcord.exe
UninstallDisplayName=rcord
LicenseFile=..\..\LICENSE
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
CloseApplications=force
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#Source}\rcord.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\rcord-webview.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\rcord"; Filename: "{app}\rcord.exe"
Name: "{userdesktop}\rcord"; Filename: "{app}\rcord.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\rcord.exe"; Description: "{cm:LaunchProgram,rcord}"; Flags: nowait postinstall skipifsilent
; rcord's own updater passes /RESTARTRCORD=1 so the new version starts after a silent upgrade.
Filename: "{app}\rcord.exe"; Flags: nowait; Check: RestartRequested

[Code]
function RestartRequested: Boolean;
begin
  Result := ExpandConstant('{param:RESTARTRCORD|0}') = '1';
end;
