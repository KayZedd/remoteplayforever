#define MyAppName "RemotePlayForever"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "RemotePlayForever"
#define MyAppURL "https://github.com/KayZedd/remoteplayforever"
#define MyAppExeName "remoteplayforever.exe"

[Setup]
AppId={{E68A8D2F-1A5F-4E90-95B3-441BF884E901}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
AllowNoIcons=yes
LicenseFile=..\LICENSE
OutputDir=..\dist
OutputBaseFilename=RemotePlayForever-Setup-{#MyAppVersion}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
UninstallDisplayIcon={app}\{#MyAppExeName}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "polish"; MessagesFile: "compiler:Languages\Polish.isl"

[CustomMessages]
english.ContextMenuTask=Add "Launch with Steam Remote Play" to the right-click menu of .exe files
polish.ContextMenuTask=Dodaj "Uruchom z Steam Remote Play" do menu pod prawym przyciskiem na plikach .exe
english.ContextMenuGroup=System integration:
polish.ContextMenuGroup=Integracja z systemem:
english.ContextMenuLabel=Launch with Steam Remote Play
polish.ContextMenuLabel=Uruchom z Steam Remote Play

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: checkedonce
Name: "contextmenu"; Description: "{cm:ContextMenuTask}"; GroupDescription: "{cm:ContextMenuGroup}"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Registry]
Root: HKA; Subkey: "Software\Classes\exefile\shell\RemotePlayForever"; ValueType: string; ValueName: ""; ValueData: "{cm:ContextMenuLabel}"; Tasks: contextmenu; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\exefile\shell\RemotePlayForever"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#MyAppExeName}"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\exefile\shell\RemotePlayForever\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" --run ""%1"""; Tasks: contextmenu

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
