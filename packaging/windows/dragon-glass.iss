; Inno Setup script for Dragon Glass.
;
; Build the binaries first (`cargo build --release -p dragonglass-ui -p dragonglass-cli`),
; then compile this with `iscc packaging\windows\dragon-glass.iss`.
;
; The CLI is shipped alongside the app so the git, export, MCP and plugin commands are
; available from a terminal after install.

#define AppName "Dragon Glass"
#define AppVersion "0.0.1"
#define AppPublisher "Dragon Glass contributors"
#define AppExe "dragon-glass-app.exe"
#define CliExe "dragon-glass.exe"

[Setup]
AppId={{8C0E3F6A-2B7E-4C1D-9F3A-7A5B6C1D2E3F}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
OutputDir=..\..\dist
OutputBaseFilename=dragon-glass-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayName={#AppName}
LicenseFile=..\..\LICENSE

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Options:"

[Files]
Source: "..\..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\{#CliExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; Register the dragonglass:// protocol so links open in the app.
Root: HKCU; Subkey: "Software\Classes\dragonglass"; ValueType: string; ValueName: ""; ValueData: "URL:Dragon Glass protocol"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\dragonglass"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\dragonglass\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent
