; MyIsle — Inno Setup 安装脚本（按用户安装，无需管理员）
; CI 用法：iscc /DAppVersion=0.2.0 installer\MyIsle.iss
#ifndef AppVersion
  #define AppVersion "0.2.0"
#endif
#define AppName "MyIsle"
#define AppExe "MyIsle.exe"

[Setup]
AppId={{C2E5A7B4-9D31-4F68-8A1E-3B7C9D2F5A61}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=MyIsle
DefaultDirName={localappdata}\Programs\MyIsle
DefaultGroupName=MyIsle
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\dist
OutputBaseFilename=MyIsle-Setup-{#AppVersion}
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
Name: "autostart"; Description: "开机自动启动 MyIsle"; GroupDescription: "附加任务:"; Flags: unchecked

[Files]
Source: "..\target\release\myisle.exe"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion

[Icons]
Name: "{group}\MyIsle"; Filename: "{app}\{#AppExe}"
Name: "{group}\卸载 MyIsle"; Filename: "{uninstallexe}"
Name: "{userdesktop}\MyIsle"; Filename: "{app}\{#AppExe}"; Tasks: autostart

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "MyIsle"; ValueData: """{app}\{#AppExe}"""; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{app}\{#AppExe}"; Description: "运行 MyIsle"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "taskkill.exe"; Parameters: "/IM {#AppExe} /F"; Flags: runhidden; RunOnceId: "KillMyIsle"
