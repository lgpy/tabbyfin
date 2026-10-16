; Windows installer (Inno Setup 6), built by packaging/package.sh from the same files as the
; .zip:
;
;   ISCC.exe /DAppVersion=0.1.0 /DSourceDir=<staged app folder> /DOutputDir=<out> tabbyfin.iss
;
; Installs for the current user by default (no admin prompt, into %LOCALAPPDATA%\Programs),
; with a Start-menu entry, an optional desktop icon, and an uninstaller in Settings -> Apps.
; Settings and sign-ins live in %APPDATA% and stay when it's uninstalled.

#ifndef AppVersion
  #error Pass /DAppVersion=<version>
#endif
#ifndef SourceDir
  #error Pass /DSourceDir=<folder with tabbyfin.exe, libmpv-2.dll and ANGLE>
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif

[Setup]
; Fixed for good: it's how upgrades find the installed copy.
AppId={{B1202137-F00E-48F6-9169-9B7EA09F23EC}
AppName=Tabbyfin
AppVersion={#AppVersion}
AppVerName=Tabbyfin {#AppVersion}
AppPublisher=lgpy
AppPublisherURL=https://github.com/lgpy/jelly-gpui
AppSupportURL=https://github.com/lgpy/jelly-gpui/issues
DefaultDirName={autopf}\Tabbyfin
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=tabbyfin-{#AppVersion}-windows-x86_64-setup
SetupIconFile=tabbyfin.ico
UninstallDisplayIcon={app}\tabbyfin.ico
UninstallDisplayName=Tabbyfin
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
; Closes a running Tabbyfin (and its player process) before replacing its files.
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[InstallDelete]
; The app was called jelly-gpui before. The same AppId upgrades that install in place,
; so its old executable, icon and shortcuts go.
Type: files; Name: "{app}\jelly-gpui.exe"
Type: files; Name: "{app}\jelly-gpui.ico"
Type: files; Name: "{autoprograms}\jelly-gpui.lnk"
Type: files; Name: "{autodesktop}\jelly-gpui.lnk"

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs
Source: "tabbyfin.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Tabbyfin"; Filename: "{app}\tabbyfin.exe"; IconFilename: "{app}\tabbyfin.ico"
Name: "{autodesktop}\Tabbyfin"; Filename: "{app}\tabbyfin.exe"; IconFilename: "{app}\tabbyfin.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\tabbyfin.exe"; Description: "{cm:LaunchProgram,Tabbyfin}"; Flags: nowait postinstall skipifsilent
