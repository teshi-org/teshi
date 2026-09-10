; Teshi per-user Windows setup. In-app updates re-run this installer with /update=true.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceRoot
  #define SourceRoot "..\staging\exe-root"
#endif
#ifndef OutputDir
  #define OutputDir "..\target\inno"
#endif
#ifndef OutputName
  #define OutputName "teshi-setup"
#endif
#ifndef CliOnly
  #define CliOnly 0
#endif

#if CliOnly
  #define MainExe "teshi.exe"
#else
  #define MainExe "teshi-desktop.exe"
#endif

[Setup]
AppId={{6D2F8E91-4B17-4C3A-A8E2-7F1B9C04D5E8}
AppName=teshi
AppVersion={#AppVersion}
AppPublisher=teshi-org
AppPublisherURL=https://github.com/teshi-org/teshi
DefaultDirName={localappdata}\Programs\teshi
DefaultGroupName=teshi
DisableProgramGroupPage=yes
DisableDirPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
Compression=lzma
SolidCompression=yes
WizardStyle=modern
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
UninstallDisplayIcon={app}\bin\{#MainExe}
ChangesEnvironment=yes
CloseApplications=no
UsePreviousAppDir=yes
MinVersion=10.0
AllowNoIcons=yes
SetupMutex=TeshiSetupMutex

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "{#SourceRoot}\*"; DestDir: "{code:GetInstallDir}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
#if CliOnly
Name: "{autoprograms}\teshi CLI"; Filename: "{app}\bin\teshi.exe"; Check: not IsUpdating
#else
Name: "{autoprograms}\teshi CLI"; Filename: "{app}\bin\teshi.exe"; Check: not IsUpdating
Name: "{autoprograms}\teshi"; Filename: "{app}\bin\teshi-desktop.exe"; Check: not IsUpdating
#endif

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}\bin"; Flags: preservestringtype noerror; Check: NeedsAddPath and not IsUpdating

[Run]
#if CliOnly
#else
Filename: "{app}\bin\teshi-desktop.exe"; Description: "Launch teshi"; Flags: nowait postinstall skipifsilent; Check: not IsUpdating
#endif

[UninstallDelete]
Type: filesandordirs; Name: "{app}\install"
Type: filesandordirs; Name: "{app}\.teshi-update"

[Code]
function IsUpdating(): Boolean;
begin
  Result := CompareText(ExpandConstant('{param:update|false}'), 'true') = 0;
end;

function GetInstallDir(Param: string): string;
begin
  if IsUpdating() then
    Result := ExpandConstant('{app}\install')
  else
    Result := ExpandConstant('{app}');
end;

function NeedsAddPath(): Boolean;
var
  OrigPath: string;
  AppBin: string;
begin
  AppBin := ExpandConstant('{app}\bin');
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', OrigPath) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(AppBin) + ';', ';' + Uppercase(OrigPath) + ';') = 0;
end;
