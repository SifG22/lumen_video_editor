; Instalador de Lumen Video Editor para Windows (Inno Setup 6). Lo genera
; packaging\lumen\empaquetar.ps1 -Instalador, que pasa la version, la arquitectura y la carpeta ya
; preparada:
;   ISCC.exe /DVersion=0.0.1 /DArch=x64 /DSource=..\..\dist\LumenVideo-0.0.1-windows-x64 /DOutDir=..\..\dist lumen-video.iss
;
; Se instala para el usuario actual (sin pedir administrador) en %LOCALAPPDATA%\Programs\Lumen Video;
; quien tenga permisos puede elegir "para todos los usuarios" y va a Archivos de programa. Crea el
; acceso en el menu Inicio (y en el escritorio si se marca), abre los .lumenv (y los .fcproj) con
; doble clic y se desinstala desde Configuracion > Aplicaciones.

#ifndef Version
  #define Version "0.0.1"
#endif
#ifndef Arch
  #define Arch "x64"
#endif
#ifndef Source
  #define Source "..\..\dist\LumenVideo-" + Version + "-windows-" + Arch
#endif
#ifndef OutDir
  #define OutDir "..\..\dist"
#endif

[Setup]
; El AppId identifica a Lumen Video para actualizar y desinstalar: no cambiarlo nunca.
AppId={{9B3E5C71-2D4A-4F86-A1C7-6E0D8F2B4A93}
AppName=Lumen Video Editor
AppVersion={#Version}
AppVerName=Lumen Video Editor {#Version}
AppPublisher=SifG22
AppPublisherURL=https://sifg.net
AppSupportURL=https://github.com/SifG22/lumen_video_editor/issues
AppUpdatesURL=https://github.com/SifG22/lumen_video_editor/releases
DefaultDirName={autopf}\Lumen Video
DefaultGroupName=Lumen Video Editor
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
#if Arch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif
MinVersion=10.0.17763
SetupIconFile=..\..\assets\app-icon\lumen.ico
UninstallDisplayIcon={app}\LumenVideo.exe
UninstallDisplayName=Lumen Video Editor
LicenseFile={#Source}\TERMINOS.txt
OutputDir={#OutDir}
OutputBaseFilename=LumenVideo-Setup-{#Version}-{#Arch}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes
CloseApplications=yes
VersionInfoVersion={#Version}
VersionInfoCompany=SifG22
VersionInfoDescription=Instalador de Lumen Video Editor
VersionInfoProductName=Lumen Video Editor

[Languages]
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#Source}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Lumen Video Editor"; Filename: "{app}\LumenVideo.exe"; AppUserModelID: "SifG22.LumenVideo"
Name: "{autodesktop}\Lumen Video Editor"; Filename: "{app}\LumenVideo.exe"; Tasks: desktopicon

[Registry]
; Los proyectos .lumenv (y los .fcproj) se abren con Lumen Video, que tambien aparece en
; "Abrir con" para los videos habituales.
Root: HKA; Subkey: "Software\Classes\.lumenv"; ValueType: string; ValueName: ""; ValueData: "LumenVideo.Proyecto"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.fcproj\OpenWithProgids"; ValueType: string; ValueName: "LumenVideo.Proyecto"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\LumenVideo.Proyecto"; ValueType: string; ValueName: ""; ValueData: "Proyecto de Lumen Video Editor"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\LumenVideo.Proyecto\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\LumenVideo.exe,0"
Root: HKA; Subkey: "Software\Classes\LumenVideo.Proyecto\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\LumenVideo.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\LumenVideo.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Lumen Video Editor"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\LumenVideo.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\LumenVideo.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\LumenVideo.exe\SupportedTypes"; ValueType: string; ValueName: ".lumenv"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\LumenVideo.exe\SupportedTypes"; ValueType: string; ValueName: ".fcproj"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\LumenVideo.exe\SupportedTypes"; ValueType: string; ValueName: ".mp4"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\LumenVideo.exe\SupportedTypes"; ValueType: string; ValueName: ".mov"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\LumenVideo.exe\SupportedTypes"; ValueType: string; ValueName: ".mkv"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\LumenVideo.exe\SupportedTypes"; ValueType: string; ValueName: ".webm"; ValueData: ""

[Run]
Filename: "{app}\LumenVideo.exe"; Description: "{cm:LaunchProgram,Lumen Video Editor}"; Flags: nowait postinstall skipifsilent
