; Modeloid's Windows installer (Inno Setup 6).
; Input:  dist\Modeloid-<version>-windows-x64\  (the packaged folder)
; Output: dist\Modeloid-<version>-setup.exe
; Build:  ISCC.exe /DAppVersion=0.1.0 installer\windows\modeloid.iss

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#define AppName "Modeloid"
#define Package "..\..\dist\Modeloid-" + AppVersion + "-windows-x64"

[Setup]
AppId={{fc566c80-8e8d-4061-aeb0-6ec8fd5da934}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=denizhoroz
AppPublisherURL=https://github.com/denizhoroz/modeloid
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\dist
OutputBaseFilename=Modeloid-{#AppVersion}-setup
SetupIconFile=..\..\modeloid-gui\assets\favicon.ico
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\Modeloid.ico
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Package}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\..\modeloid-gui\assets\favicon.ico"; DestDir: "{app}"; DestName: "Modeloid.ico"

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\Modeloid.exe"; WorkingDir: "{app}"; IconFilename: "{app}\Modeloid.ico"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\Modeloid.exe"; WorkingDir: "{app}"; IconFilename: "{app}\Modeloid.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\Modeloid.exe"; Description: "{cm:LaunchProgram,{#AppName}}"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent

[Code]
// After uninstalling: offer to delete the Python environment, which the app
// downloaded (not the installer), so the uninstaller doesn't know its files.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  EnvDir: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    EnvDir := ExpandConstant('{localappdata}\Modeloid');
    if DirExists(EnvDir) and not UninstallSilent() then
      if MsgBox('Also remove Modeloid''s Python environment?' + #13#10 + EnvDir + ' (about 1-3 GB)' + #13#10#13#10 +
                'Your workspaces and datasets are not affected.',
                mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(EnvDir, True, True, True);
  end;
end;

