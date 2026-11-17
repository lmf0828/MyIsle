; 灵岛 i岛 — Inno Setup 安装脚本（按用户安装，无需管理员）
; CI 用法：iscc /DAppVersion=0.2.0 installer\LingDao.iss
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#define AppName "i岛 LingDao"
#define AppExe "LingDao.exe"

[Setup]
AppId={{8F3A2C1E-6B4D-4A79-9C2B-7E5D1F0A3B84}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=LingDao
DefaultDirName={localappdata}\Programs\LingDao
DefaultGroupName=i岛
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\dist
OutputBaseFilename=LingDao-Setup-{#AppVersion}
SetupIconFile=..\assets\island.ico
UninstallDisplayIcon={app}\{#AppExe}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=yes
MinVersion=10.0

[Languages]
Name: "chinesesimp"; MessagesFile: "ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "开机自动启动 i岛"; GroupDescription: "附加任务:"; Flags: unchecked

[Files]
Source: "..\target\release\lingdao.exe"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion

[Icons]
Name: "{group}\i岛"; Filename: "{app}\{#AppExe}"
Name: "{group}\卸载 i岛"; Filename: "{uninstallexe}"
Name: "{userdesktop}\i岛"; Filename: "{app}\{#AppExe}"; Tasks: autostart

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "LingDao"; ValueData: """{app}\{#AppExe}"""; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{app}\{#AppExe}"; Description: "运行 i岛"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "taskkill.exe"; Parameters: "/IM {#AppExe} /F"; Flags: runhidden; RunOnceId: "KillLingDao"
