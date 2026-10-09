; Inno Setup script: builds a Windows installer from dist\SoundTune.
; Run packaging/windows/package.sh first, then:
;   iscc packaging\windows\soundtune.iss
; Override the version with /DAppVersion=1.2.3 (CI does this for tags).

#define AppName "SoundTune"
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

[Setup]
AppId={{6F1B5B7E-2C1A-4C6E-9A8E-5D3B7F0A1C42}
AppName={#AppName}
AppVersion={#AppVersion}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
OutputDir=..\..\dist
OutputBaseFilename=SoundTune-setup-x64
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequiredOverridesAllowed=dialog
UninstallDisplayIcon={app}\bin\soundtune.exe

[Files]
Source: "..\..\dist\SoundTune\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\bin\soundtune.exe"; WorkingDir: "{app}\bin"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\bin\soundtune.exe"; WorkingDir: "{app}\bin"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Run]
Filename: "{app}\bin\soundtune.exe"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent
