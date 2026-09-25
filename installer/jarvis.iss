; Jarvis installer (Inno Setup 6). Built by CI from dist\Jarvis:
;   ISCC.exe /DAppVersion=0.2.0 installer\jarvis.iss   ->   dist\JarvisSetup.exe
; Per-user install (no admin rights). The voice server (Whisper + voice clone) is an
; optional task that downloads Python and 3-7 GB of packages and models for the detected
; graphics card (NVIDIA CUDA, AMD ROCm/Vulkan, Intel Vulkan or the CPU; see tools\voice-server\gpu.py).

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6F1C2B7E-4D3A-4B8E-9E57-2A1D5C0B7F31}
AppName=Джарвис
AppVersion={#AppVersion}
AppPublisher=softlyfear (форк Priler/jarvis)
AppPublisherURL=https://github.com/softlyfear/jarvis
AppSupportURL=https://github.com/softlyfear/jarvis/issues
DefaultDirName=C:\Jarvis
UsePreviousAppDir=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=JarvisSetup
SetupIconFile=..\resources\icons\icon.ico
UninstallDisplayIcon={app}\jarvis-app.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=force
RestartApplications=no

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "voice"; Description: "Точное распознавание речи (Whisper) и голос Джарвиса для ответов — под вашу видеокарту (NVIDIA, AMD или Intel), скачивается 3–7 ГБ, 20–40 минут"
Name: "autostart"; Description: "Запускать Джарвиса вместе с Windows"
Name: "desktopicon"; Description: "Ярлык на рабочем столе"

[Files]
Source: "..\dist\Jarvis\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "configure.ps1"; DestDir: "{app}\installer"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Джарвис"; Filename: "{app}\jarvis-app.exe"; WorkingDir: "{app}"
Name: "{autoprograms}\Джарвис — окно и настройки"; Filename: "{app}\jarvis-gui.exe"; WorkingDir: "{app}"
Name: "{autoprograms}\Джарвис — ключи и параметры (assistant.toml)"; Filename: "notepad.exe"; Parameters: """{userappdata}\com.priler.jarvis\assistant.toml"""
Name: "{autodesktop}\Джарвис"; Filename: "{app}\jarvis-app.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[INI]
Filename: "{autoprograms}\Джарвис — инструкция.url"; Section: "InternetShortcut"; Key: "URL"; String: "https://github.com/softlyfear/jarvis/blob/master/docs/INSTALL-RU.md"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Jarvis"; ValueData: """{app}\jarvis-app.exe"""; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\installer\configure.ps1"" -Template ""{app}\assistant.example.toml"" -KeysFile ""{tmp}\kilo-key.txt"" {code:VoiceFlag} {code:AddressFlag}"; Flags: runhidden waituntilterminated; StatusMsg: "Сохранение настроек..."
; the voice server (install.ps1) is installed from [Code] (RunVoiceInstall) with a progress page, without a console window
Filename: "{app}\jarvis-app.exe"; Description: "Запустить Джарвиса"; WorkingDir: "{app}"; Flags: postinstall nowait skipifsilent
; silent run = update from the app: start Jarvis and its window again
Filename: "{app}\jarvis-app.exe"; WorkingDir: "{app}"; Flags: nowait; Check: WizardSilent
Filename: "{app}\jarvis-gui.exe"; WorkingDir: "{app}"; Flags: nowait; Check: WizardSilent
Filename: "{app}\jarvis-gui.exe"; Description: "Открыть окно с шаром"; WorkingDir: "{app}"; Flags: postinstall nowait skipifsilent unchecked

[UninstallRun]
; stop Jarvis and the voice server (python.exe inside {app}) before removing files
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -Command ""Get-CimInstance Win32_Process | Where-Object {{ $_.ExecutablePath -like '{app}\*' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}"""; Flags: runhidden waituntilterminated; RunOnceId: "StopJarvis"

[UninstallDelete]
Type: files; Name: "{autoprograms}\Джарвис — инструкция.url"
Type: filesandordirs; Name: "{app}\tools\voice-server\.venv"
Type: filesandordirs; Name: "{app}\tools\voice-server\python"
Type: filesandordirs; Name: "{app}\tools\voice-server\models"
Type: filesandordirs; Name: "{app}\tools\voice-server\__pycache__"
Type: files; Name: "{app}\tools\voice-server\gpu-profile.json"

[Code]
const
  KiloKeyUrl = 'https://app.kilo.ai';
  PolzaKeyUrl = 'https://polza.ai';

var
  KeysPage: TInputQueryWizardPage;
  AddressPage: TInputOptionWizardPage;
  KeyLink: TNewStaticText;
  KeyButton: TNewButton;
  KiloButton: TNewButton;
  KeyHint: TNewStaticText;
  GpuLabel: TNewStaticText;
  VoicePage: TOutputMarqueeProgressWizardPage;
  VoiceStep: String;
  VoiceLines: TArrayOfString;

// the display adapter the voice server will use: NVIDIA, then a Radeon card, then any AMD/Intel GPU
function DetectGpu(var Vendor: String): String;
var
  Locator, Service, Items, Item: Variant;
  I, Rank, BestRank: Integer;
  Name, Pnp: String;
begin
  Result := '';
  Vendor := '';
  BestRank := 0;
  try
    Locator := CreateOleObject('WbemScripting.SWbemLocator');
    Service := Locator.ConnectServer('.', 'root\CIMV2');
    Items := Service.ExecQuery('SELECT Name, PNPDeviceID FROM Win32_VideoController');
    for I := 0 to Items.Count - 1 do
    begin
      Item := Items.ItemIndex(I);
      Name := '';
      Pnp := '';
      // Variant -> String by assignment: Trim/Uppercase do not accept a Variant
      if not VarIsNull(Item.Name) then Name := Item.Name;
      if not VarIsNull(Item.PNPDeviceID) then Pnp := Item.PNPDeviceID;
      Name := Trim(Name);
      Pnp := Uppercase(Pnp);
      Rank := 0;
      if Pos('VEN_10DE', Pnp) > 0 then Rank := 30
      else if (Pos('VEN_1002', Pnp) > 0) and (Pos(' RX ', ' ' + Uppercase(Name) + ' ') > 0) then Rank := 25
      else if Pos('VEN_1002', Pnp) > 0 then Rank := 20
      else if Pos('VEN_8086', Pnp) > 0 then Rank := 10;
      if Rank > BestRank then
      begin
        BestRank := Rank;
        Result := Name;
        if Rank = 30 then Vendor := 'nvidia'
        else if Rank >= 20 then Vendor := 'amd'
        else Vendor := 'intel';
      end;
    end;
  except
    Result := '';
    Vendor := '';
  end;
end;

function GpuSummary: String;
var
  Vendor, Name: String;
begin
  Name := DetectGpu(Vendor);
  if Vendor = 'nvidia' then
    Result := 'Видеокарта: ' + Name + '. Распознавание и голос будут работать на ней (CUDA).'
  else if Vendor = 'amd' then
    Result := 'Видеокарта: ' + Name + '. Распознавание — на ней (Vulkan); голос — на ней через ROCm ' +
      '(Radeon RX 5000 и новее, нужен свежий драйвер Adrenalin), иначе на процессоре.'
  else if Vendor = 'intel' then
    Result := 'Видеокарта: ' + Name + '. Распознавание — на ней (Vulkan), голос — на процессоре (ответ медленнее).'
  else
    Result := 'Видеокарта не найдена: распознавание и голос будут на процессоре (медленно, лучше оставить голос Windows).';
end;

procedure OpenKeysSite(Sender: TObject);
var
  ErrorCode: Integer;
begin
  ShellExecAsOriginalUser('open', PolzaKeyUrl, '', '', SW_SHOWNORMAL, ewNoWait, ErrorCode);
end;

procedure OpenKiloSite(Sender: TObject);
var
  ErrorCode: Integer;
begin
  ShellExecAsOriginalUser('open', KiloKeyUrl, '', '', SW_SHOWNORMAL, ewNoWait, ErrorCode);
end;

function IsAscii(const S: String): Boolean;
var
  I: Integer;
begin
  Result := True;
  for I := 1 to Length(S) do
    if Ord(S[I]) > 127 then
    begin
      Result := False;
      Exit;
    end;
end;

procedure InitializeWizard;
begin
  KeysPage := CreateInputQueryPage(wpSelectTasks,
    'Нейросеть (необязательно)',
    'Для разговора и сложных просьб, которых нет среди команд.',
    'Без VPN: ключ Polza AI («sk-polza-…», оплата рублями от 100 ₽, просьба стоит несколько копеек). ' +
    'С VPN подойдёт и ключ Kilo («eyJ…»); без ключа Kilo даёт бесплатные модели, но из России только через VPN. ' +
    'Шлюз определяется по ключу. Можно оставить пустым и добавить позже в настройках Джарвиса.');
  KeysPage.Add('Ключ Polza AI или Kilo:', False);

  // clickable link and a button under the key field
  KeyLink := TNewStaticText.Create(KeysPage);
  KeyLink.Parent := KeysPage.Surface;
  KeyLink.Caption := 'Где взять ключ: ' + PolzaKeyUrl;
  KeyLink.Cursor := crHand;
  KeyLink.Font.Color := clBlue;
  KeyLink.Font.Style := [fsUnderline];
  KeyLink.Top := KeysPage.Edits[0].Top + KeysPage.Edits[0].Height + ScaleY(12);
  KeyLink.Left := KeysPage.Edits[0].Left;
  KeyLink.OnClick := @OpenKeysSite;

  KeyButton := TNewButton.Create(KeysPage);
  KeyButton.Parent := KeysPage.Surface;
  KeyButton.Caption := 'Ключ Polza AI (без VPN)';
  KeyButton.Width := ScaleX(190);
  KeyButton.Height := ScaleY(26);
  KeyButton.Top := KeyLink.Top + KeyLink.Height + ScaleY(8);
  KeyButton.Left := KeysPage.Edits[0].Left;
  KeyButton.OnClick := @OpenKeysSite;

  KiloButton := TNewButton.Create(KeysPage);
  KiloButton.Parent := KeysPage.Surface;
  KiloButton.Caption := 'Ключ Kilo (нужен VPN)';
  KiloButton.Width := ScaleX(190);
  KiloButton.Height := ScaleY(26);
  KiloButton.Top := KeyButton.Top;
  KiloButton.Left := KeyButton.Left + KeyButton.Width + ScaleX(8);
  KiloButton.OnClick := @OpenKiloSite;

  KeyHint := TNewStaticText.Create(KeysPage);
  KeyHint.Parent := KeysPage.Surface;
  KeyHint.Caption := 'Polza AI: личный кабинет → ключи API (начинается с sk-). Kilo: app.kilo.ai → Your Profile → внизу страницы (начинается с eyJ).';
  KeyHint.AutoSize := False;
  KeyHint.WordWrap := True;
  KeyHint.Width := KeysPage.SurfaceWidth - KeysPage.Edits[0].Left;
  KeyHint.Height := ScaleY(32);
  KeyHint.Top := KeyButton.Top + KeyButton.Height + ScaleY(8);
  KeyHint.Left := KeysPage.Edits[0].Left;

  AddressPage := CreateInputOptionPage(KeysPage.ID,
    'Обращение',
    'Как Джарвису к вам обращаться?',
    'Так к вам будет обращаться нейросеть, а с голосовым сервером — и короткие отклики Джарвиса ' +
    '(«Слушаю, мисс»). Поменять можно в настройках: Джарвис → «Как Джарвис к вам обращается».',
    True, False);
  AddressPage.Add('Сэр');
  AddressPage.Add('Мисс');
  AddressPage.SelectedValueIndex := 0;

  // what the voice task will use, under the task list
  GpuLabel := TNewStaticText.Create(WizardForm);
  GpuLabel.Parent := WizardForm.SelectTasksPage;
  GpuLabel.AutoSize := False;
  GpuLabel.WordWrap := True;
  GpuLabel.Left := WizardForm.TasksList.Left;
  GpuLabel.Width := WizardForm.TasksList.Width;
  GpuLabel.Height := ScaleY(44);
  WizardForm.TasksList.Height := WizardForm.TasksList.Height - GpuLabel.Height - ScaleY(8);
  GpuLabel.Top := WizardForm.TasksList.Top + WizardForm.TasksList.Height + ScaleY(8);
  GpuLabel.Caption := GpuSummary;

  VoicePage := CreateOutputMarqueeProgressPage('Распознавание речи и голос Джарвиса',
    'Скачивание и настройка под вашу видеокарту: 20–40 минут, зависит от интернета. Не закрывайте установщик.');
end;

// an update replaces files that Jarvis, its voice server and whisper-server keep open
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  Result := '';
  if FileExists(ExpandConstant('{app}\jarvis-app.exe')) then
    Exec('powershell.exe', '-NoProfile -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | ' +
      'Where-Object { $_.ExecutablePath -like ''' + ExpandConstant('{app}') + '\*'' } | ' +
      'ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }"',
      '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (CurPageID = wpSelectDir) and not IsAscii(WizardDirValue) then
  begin
    MsgBox('Путь к папке должен состоять только из латинских букв и цифр, например C:\Jarvis.' + #13#10 +
      'Распознавание речи не открывает модели по путям с русскими буквами.', mbError, MB_OK);
    Result := False;
  end;
end;

function AddressFlag(Param: String): String;
begin
  // a silent update keeps the address from the settings
  if WizardSilent then
    Result := ''
  else if AddressPage.SelectedValueIndex = 1 then
    Result := '-Address miss'
  else
    Result := '-Address sir';
end;

function VoiceFlag(Param: String): String;
begin
  // a silent update must not override the voice the user picked in settings
  if WizardIsTaskSelected('voice') and not WizardSilent then
    Result := '-VoiceClone'
  else
    Result := '';
end;

// output of install.ps1, line by line: "==> step" lines become the heading,
// pip's "Progress N of M" lines become megabytes and percent
procedure VoiceLog(const S: String; const Error, FirstLine: Boolean);
var
  Line: String;
  P, N: Integer;
  Current, Total: Int64;
begin
  Line := Trim(S);
  if Line = '' then
    Exit;
  if Copy(Line, 1, 9) = 'Progress ' then
  begin
    P := Pos(' of ', Line);
    Current := StrToInt64Def(Copy(Line, 10, P - 10), 0);
    Total := StrToInt64Def(Copy(Line, P + 4, Length(Line)), 0);
    if Total > 0 then
      VoicePage.SetText(VoiceStep, 'Скачано ' + IntToStr(Current div 1048576) + ' из ' +
        IntToStr(Total div 1048576) + ' МБ (' + IntToStr((Current * 100) div Total) + '%)');
    Exit;
  end;
  N := GetArrayLength(VoiceLines);
  SetArrayLength(VoiceLines, N + 1);
  VoiceLines[N] := Line;
  if Copy(Line, 1, 4) = '==> ' then
    VoiceStep := Copy(Line, 5, Length(Line));
  VoicePage.SetText(VoiceStep, Line);
end;

procedure RunVoiceInstall;
var
  Ok: Boolean;
  Code: Integer;
  LogDir: String;
begin
  VoiceStep := 'Подготовка';
  SetArrayLength(VoiceLines, 0);
  VoicePage.SetText(VoiceStep, '');
  VoicePage.Show;
  try
    try
      Ok := ExecAndLogOutput('powershell.exe',
        '-NoProfile -ExecutionPolicy Bypass -File "' + ExpandConstant('{app}\tools\voice-server\install.ps1') + '" -NoPause -Installer',
        ExpandConstant('{app}\tools\voice-server'), SW_SHOWNORMAL, ewWaitUntilTerminated, Code, @VoiceLog);
    except
      Ok := False;
      Code := -1;
      VoiceLog(GetExceptionMessage, True, False);
    end;
  finally
    VoicePage.Hide;
  end;
  // the whole output for bug reports: "Собрать логи" in the app picks up *.log from this folder
  LogDir := ExpandConstant('{userappdata}\com.priler.jarvis');
  ForceDirectories(LogDir);
  SaveStringsToUTF8File(LogDir + '\voice-install.log', VoiceLines, False);
  if (not Ok) or (Code <> 0) then
    MsgBox('Установка распознавания и голоса не завершилась (код ' + IntToStr(Code) + ').' + #13#10 +
      'Джарвис работает и без неё. Подробности — voice-install.log в папке настроек (кнопка «Собрать логи»).' + #13#10 +
      'Повторить: ' + ExpandConstant('{app}\tools\voice-server\setup.bat'), mbError, MB_OK);
end;

// settings, the Kilo key and logs (%APPDATA%) and the window's WebView data (%LOCALAPPDATA%)
// live outside {app}: removed only when the user agrees, so a reinstall can keep them
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
    if MsgBox('Удалить также настройки Джарвиса, ключ нейросети и журналы?' + #13#10 +
      'Нажмите «Нет», если собираетесь установить Джарвиса снова.', mbConfirmation, MB_YESNO) = IDYES then
    begin
      DelTree(ExpandConstant('{userappdata}\com.priler.jarvis'), True, True, True);
      DelTree(ExpandConstant('{localappdata}\com.priler.jarvis'), True, True, True);
    end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Keys: String;
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('voice') and not WizardSilent then
    RunVoiceInstall;
  // written before [Run]; configure.ps1 reads and deletes it, keys never go on a command line
  if CurStep = ssInstall then
  begin
    Keys := Trim(KeysPage.Values[0]);
    if Keys <> '' then
      SaveStringToFile(ExpandConstant('{tmp}\kilo-key.txt'), Keys, False);
  end;
end;
