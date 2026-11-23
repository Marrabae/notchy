; Installer notchy (Inno Setup 6)
; Build: D:\Tools\InnoSetup\ISCC.exe installer\notchy.iss
; Hasil: dist\notchy-setup-<versi>.exe

#define AppName "notchy"
#define AppVersion "0.2.7"
#define AppExe "notchy.exe"
#define BuildDir "D:\Ravelware\cargo-target\notchy\release"

[Setup]
AppId={{6F2C7A1E-4B8D-4E7B-9A3C-2D5E8F1B0C47}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppName}
; per-user, tanpa admin
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\{#AppName}
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename=notchy-setup-{#AppVersion}
SetupIconFile=..\assets\notchy.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Tasks]
Name: "startup"; Description: "Jalankan notchy otomatis saat Windows startup"; GroupDescription: "Opsi:"
Name: "desktopicon"; Description: "Buat shortcut di Desktop"; GroupDescription: "Opsi:"; Flags: unchecked

[Files]
Source: "{#BuildDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; startup per-user (bisa dimatikan juga dari Settings > Apps > Startup)
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#AppName}"; ValueData: """{app}\{#AppExe}"""; Flags: uninsdeletevalue; Tasks: startup

[Run]
Filename: "{app}\{#AppExe}"; Description: "Jalankan notchy sekarang"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM {#AppExe}"; Flags: runhidden; RunOnceId: "KillNotchy"

[Code]
procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
begin
  // tutup notchy yang sedang jalan sebelum exe ditimpa (update / reinstall)
  if CurStep = ssInstall then
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM {#AppExe}', '', SW_HIDE, ewWaitUntilTerminated, Code);
  // reinstall tanpa centang startup -> hapus entri startup lama
  if (CurStep = ssPostInstall) and not WizardIsTaskSelected('startup') then
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', '{#AppName}');
end;
