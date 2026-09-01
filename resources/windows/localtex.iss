#define MyAppName "LocalTeX"
#ifndef MyAppVersion
  #define MyAppVersion "0.1.0"
#endif
#ifndef MyAppOutputDir
  #define MyAppOutputDir "..\\..\\target\\release"
#endif
#ifndef MyAppSourceDir
  #define MyAppSourceDir "payload"
#endif
#ifndef MyAppArch
  #define MyAppArch "x86_64"
#endif

[Setup]
; Keep AppId stable. A new GUID is a second Add/Remove Programs entry.
AppId={{4B509438-D116-4E0A-87F7-76023183DEA0}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher=Yan Tang
AppPublisherURL=https://github.com/kenanking/LocalTeX
DefaultDirName={localappdata}\Programs\LocalTeX
DefaultGroupName={#MyAppName}
PrivilegesRequired=lowest
OutputDir={#MyAppOutputDir}
OutputBaseFilename=LocalTeX-{#MyAppVersion}-{#MyAppArch}-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\localtex.exe
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "{#MyAppSourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\localtex.exe"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\localtex.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Run]
Filename: "{app}\localtex.exe"; Description: "Launch LocalTeX"; Flags: nowait postinstall skipifsilent
