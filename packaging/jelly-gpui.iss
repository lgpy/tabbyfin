; Windows installer (Inno Setup 6), built by packaging/package.sh from the same files as the
; .zip:
;
;   ISCC.exe /DAppVersion=0.1.0 /DSourceDir=<staged app folder> /DOutputDir=<out> jelly-gpui.iss
;
; Installs for the current user by default (no admin prompt, into %LOCALAPPDATA%\Programs),
; with a Start-menu entry, an optional desktop icon, and an uninstaller in Settings -> Apps.
; Settings and sign-ins live in %APPDATA% and stay when it's uninstalled.

#ifndef AppVersion
  #error Pass /DAppVersion=<version>
#endif
#ifndef SourceDir
  #error Pass /DSourceDir=<folder with jelly-gpui.exe, libmpv-2.dll and ANGLE>
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif

[Setup]
; Fixed for good: it's how upgrades find the installed copy.
AppId={{B1202137-F00E-48F6-9169-9B7EA09F23EC}
AppName=jelly-gpui
AppVersion={#AppVersion}
AppVerName=jelly-gpui {#AppVersion}
AppPublisher=lgpy
AppPublisherURL=https://github.com/lgpy/jelly-gpui
AppSupportURL=https://github.com/lgpy/jelly-gpui/issues
DefaultDirName={autopf}\jelly-gpui
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=jelly-gpui-{#AppVersion}-windows-x86_64-setup
SetupIconFile=jelly-gpui.ico
UninstallDisplayIcon={app}\jelly-gpui.ico
UninstallDisplayName=jelly-gpui
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
; Closes a running jelly-gpui (and its player process) before replacing its files.
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs
Source: "jelly-gpui.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\jelly-gpui"; Filename: "{app}\jelly-gpui.exe"; IconFilename: "{app}\jelly-gpui.ico"
Name: "{autodesktop}\jelly-gpui"; Filename: "{app}\jelly-gpui.exe"; IconFilename: "{app}\jelly-gpui.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\jelly-gpui.exe"; Description: "{cm:LaunchProgram,jelly-gpui}"; Flags: nowait postinstall skipifsilent
