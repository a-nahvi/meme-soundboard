[Setup]
AppId=MemeSoundboard.Windows
AppName=Meme Soundboard
AppVersion=0.4.7
AppPublisher=Meme Soundboard
AppPublisherURL=https://github.com/a-nahvi/meme-soundboard
AppSupportURL=https://github.com/a-nahvi/meme-soundboard/issues
AppUpdatesURL=https://github.com/a-nahvi/meme-soundboard/releases/latest
DefaultDirName={localappdata}\Programs\Meme Soundboard
DefaultGroupName=Meme Soundboard
DisableProgramGroupPage=yes
UsePreviousAppDir=yes
CloseApplications=yes
RestartApplications=no
PrivilegesRequired=lowest
MinVersion=10.0.22000
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\meme-soundboard.exe
OutputDir=dist
OutputBaseFilename=MemeSoundboard-Setup-x64
SetupIconFile=assets\app.ico
Compression=lzma2
SolidCompression=yes
ArchiveExtraction=full
WizardStyle=modern
VersionInfoVersion=0.4.7.0
VersionInfoDescription=Meme Soundboard Setup

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked
Name: "startup"; Description: "Start Soundboard in the tray when I sign in to Windows"
Name: "soundboardmic"; Description: "Install Soundboard Microphone (VAC Lite for personal use)"; Flags: unchecked

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Meme Soundboard"; ValueData: """{app}\meme-soundboard.exe"" --background"; Tasks: startup; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Meme Soundboard"; Tasks: not startup; Flags: deletevalue

[Files]
Source: "target\release\meme-soundboard.exe"; Flags: dontcopy noencryption
Source: "vendor\vac471lite.zip"; Flags: dontcopy noencryption
Source: "target\release\meme-soundboard.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "assets\app.ico"; DestDir: "{app}"; DestName: "meme-soundboard-icon-v2.ico"; Flags: ignoreversion
Source: "assets\app.png"; DestDir: "{app}\assets"; Flags: ignoreversion
Source: "README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "assets\licenses\Lucide.txt"; DestDir: "{app}\assets\licenses"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Meme Soundboard"; Filename: "{app}\meme-soundboard.exe"; WorkingDir: "{app}"; IconFilename: "{app}\meme-soundboard-icon-v2.ico"
Name: "{userdesktop}\Meme Soundboard"; Filename: "{app}\meme-soundboard.exe"; WorkingDir: "{app}"; IconFilename: "{app}\meme-soundboard-icon-v2.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\meme-soundboard.exe"; Description: "Launch Meme Soundboard"; Flags: postinstall skipifsilent nowait

[Code]
const
  SHCNE_UPDATEITEM = $2000;
  SHCNF_PATHW = $5;
  DirectoryOrReparsePoint = $410;
  VacArchive = 'vac471lite.zip';
  VacSha256 = '1ed42a7e763bf6237438fb62191ae88b2922f014b37658b29d7801d72c9d5357';
  VacServiceKey = 'SYSTEM\CurrentControlSet\Services\VirtualAudioCable_83ed7f0e';

var
  MicrophoneRestartNeeded: Boolean;

procedure SHChangeNotify(EventId: Longint; Flags: Cardinal; Item: String; Reserved: NativeUInt);
  external 'SHChangeNotify@shell32.dll stdcall';

function CompareStringOrdinal(First: String; FirstLength: Integer; Second: String;
  SecondLength: Integer; IgnoreCase: Cardinal): Integer;
  external 'CompareStringOrdinal@kernel32.dll stdcall';

function SameWindowsPath(const First, Second: String): Boolean;
begin
  Result := CompareStringOrdinal(First, -1, Second, -1, 1) = 2;
end;

procedure NotifyShellItemChanged(const Path: String);
begin
  SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW, Path, 0);
end;

procedure RefreshPinnedIcon(const Shell: Variant; const LinkPath, ExePath, IconLocation: String);
var
  Shortcut: Variant;
begin
  try
    Shortcut := Shell.CreateShortcut(LinkPath);
    if not SameWindowsPath(Shortcut.TargetPath, ExePath) then
      Exit;
  except
    Exit;
  end;

  try
    if not SameWindowsPath(Shortcut.IconLocation, IconLocation) then
    begin
      Shortcut.IconLocation := IconLocation;
      Shortcut.Save;
    end;
    NotifyShellItemChanged(LinkPath);
  except
    Log('Could not refresh the Meme Soundboard taskbar shortcut: ' + GetExceptionMessage);
  end;
end;

procedure RefreshPinnedIcons(const PinDir, ExePath, IconPath: String);
var
  Shell: Variant;
  Link: TFindRec;
  IconLocation: String;
begin
  if not FindFirst(AddBackslash(PinDir) + '*.lnk', Link) then
    Exit;
  try
    try
      Shell := CreateOleObject('WScript.Shell');
      IconLocation := IconPath + ',0';
      repeat
        if (Link.Attributes and DirectoryOrReparsePoint) = 0 then
          RefreshPinnedIcon(Shell, AddBackslash(PinDir) + Link.Name, ExePath, IconLocation);
      until not FindNext(Link);
    except
      Log('Could not load the Windows shortcut editor: ' + GetExceptionMessage);
    end;
  finally
    FindClose(Link);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ExitCode: Integer;
begin
  Result := '';
  if not FileExists(ExpandConstant('{app}\meme-soundboard.exe')) then
    Exit;

  try
    ExtractTemporaryFile('meme-soundboard.exe');
    if Exec(ExpandConstant('{tmp}\meme-soundboard.exe'),
      '--prepare-update "' + ExpandConstant('{app}\meme-soundboard.exe') + '"', ExpandConstant('{tmp}'),
      SW_HIDE, ewWaitUntilTerminated, ExitCode) and (ExitCode = 0) then
      Exit;
    LogFmt('Soundboard shutdown helper returned %d.', [ExitCode]);
  except
    Log('Could not run the Soundboard shutdown helper: ' + GetExceptionMessage);
  end;

  Result := 'Setup could not close this installation of Meme Soundboard. Choose Quit from its tray menu and run Setup again.';
end;

function MicrophoneReady: Boolean;
var
  ExitCode: Integer;
begin
  Result := Exec(ExpandConstant('{app}\meme-soundboard.exe'), '--check-soundboard-microphone',
    ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ExitCode) and (ExitCode = 0);
end;

procedure InstallSoundboardMicrophone;
var
  ArchivePath, DriverDir, DriverSetup: String;
  ExitCode, Attempt: Integer;
begin
  if MicrophoneReady then
    Exit;

  if RegKeyExists(HKLM, VacServiceKey) then
  begin
    MsgBox('Virtual Audio Cable is already installed. Enable its playback and recording devices in Windows Sound settings, then refresh Audio devices in Soundboard.',
      mbInformation, MB_OK);
    Exit;
  end;

  try
    ExtractTemporaryFile(VacArchive);
    ArchivePath := ExpandConstant('{tmp}\') + VacArchive;
    if CompareText(GetSHA256OfFile(ArchivePath), VacSha256) <> 0 then
      RaiseException('The bundled VAC Lite package failed its integrity check.');
    DriverDir := ExpandConstant('{tmp}\soundboard-microphone');
    ExtractArchive(ArchivePath, DriverDir, '', True, nil);
    if IsArm64 then
      DriverSetup := AddBackslash(DriverDir) + 'setup64a.exe'
    else
      DriverSetup := AddBackslash(DriverDir) + 'setup64.exe';

    if ShellExec('runas', DriverSetup, '', DriverDir,
      SW_SHOWNORMAL, ewWaitUntilTerminated, ExitCode) then
    begin
      LogFmt('VAC Lite Setup returned %d.', [ExitCode]);
      if ExitCode = 3010 then
      begin
        MicrophoneRestartNeeded := True;
        Exit;
      end;
      for Attempt := 1 to 6 do
      begin
        if MicrophoneReady then
          Exit;
        Sleep(400);
      end;
    end
    else
      LogFmt('VAC Lite Setup could not start (%d).', [ExitCode]);
  except
    Log('Could not install Soundboard Microphone: ' + GetExceptionMessage);
  end;

  MsgBox('Soundboard is installed. Windows has not made the optional virtual microphone available. You can use local playback and run this installer again to install Soundboard Microphone.',
    mbInformation, MB_OK);
end;

function NeedRestart: Boolean;
begin
  Result := MicrophoneRestartNeeded;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ExePath: String;
begin
  if CurStep <> ssPostInstall then
    Exit;

  ExePath := ExpandConstant('{app}\meme-soundboard.exe');
  RefreshPinnedIcons(
    ExpandConstant('{userappdata}\Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar'),
    ExePath, ExpandConstant('{app}\meme-soundboard-icon-v2.ico'));
  NotifyShellItemChanged(ExePath);
  if WizardIsTaskSelected('soundboardmic') and not WizardSilent then
    InstallSoundboardMicrophone;
end;
