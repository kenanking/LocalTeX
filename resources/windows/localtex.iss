#define MyAppName "LocalTeX"
#ifndef MyAppVersion
  #define MyAppVersion "0.1.0"
#endif
#ifndef MyAppOutputDir
  #define MyAppOutputDir "..\\..\\dist"
#endif
#ifndef MyAppSourceDir
  #define MyAppSourceDir "payload"
#endif
#ifndef MyAppArch
  #define MyAppArch "x86_64"
#endif
#ifndef MyAppSetupIcon
  #define MyAppSetupIcon "..\\..\\target\\localtex.ico"
#endif

[Setup]
; Keep AppId stable. A new GUID is a second Add/Remove Programs entry.
AppId={{4B509438-D116-4E0A-87F7-76023183DEA0}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher=Yan Tang
AppPublisherURL=https://github.com/kenanking/LocalTeX
DefaultDirName={localappdata}\Programs\LocalTeX
DefaultGroupName={#MyAppName}
PrivilegesRequired=lowest
OutputDir={#MyAppOutputDir}
OutputBaseFilename=LocalTeX-{#MyAppVersion}-{#MyAppArch}-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\localtex.exe
SetupIconFile={#MyAppSetupIcon}
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "{#MyAppSourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\localtex.exe"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\localtex.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Run]
Filename: "{app}\localtex.exe"; Description: "Launch LocalTeX"; Flags: nowait postinstall skipifsilent

; The app owns this value (Settings -> Launch at startup, off by default).
; Delete it on uninstall so a leftover Run entry cannot point at a missing exe.
[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "LocalTeX"; Flags: dontcreatekey uninsdeletevalue

[Code]
var
  DeleteUserData: Boolean;
  DesktopCheck: TNewCheckBox;
  DesktopCheckLabel: TNewStaticText;
  TasksGroupLabel: TNewStaticText;
  LaunchCheck: TNewCheckBox;
  LaunchCheckLabel: TNewStaticText;

procedure HideNativeCheckList(List: TNewCheckListBox);
begin
  { Inno rebuilds TasksList/RunList when those pages are shown, which also
    restores Visible. Collapse them so the unscaled glyph cannot paint. }
  List.Visible := False;
  List.Height := 0;
end;

procedure SyncDesktopCheckFromTasks();
begin
  { Grouped tasks stay at ItemLevel 0; the group header is a separate item. }
  DesktopCheck.Checked := WizardIsTaskSelected('desktopicon');
end;

procedure SyncTasksFromDesktopCheck();
begin
  if DesktopCheck.Checked then
    WizardSelectTasks('desktopicon')
  else
    WizardSelectTasks('!desktopicon');
end;

procedure SyncLaunchCheckFromRun();
begin
  if WizardForm.RunList.Items.Count > 0 then
    LaunchCheck.Checked := WizardForm.RunList.Checked[0];
end;

procedure SyncRunFromLaunchCheck();
var
  I: Integer;
begin
  for I := 0 to WizardForm.RunList.Items.Count - 1 do
    WizardForm.RunList.Checked[I] := LaunchCheck.Checked;
end;

procedure DesktopCheckClick(Sender: TObject);
begin
  SyncTasksFromDesktopCheck();
end;

procedure DesktopCheckLabelClick(Sender: TObject);
begin
  DesktopCheck.Checked := not DesktopCheck.Checked;
end;

procedure LaunchCheckLabelClick(Sender: TObject);
begin
  LaunchCheck.Checked := not LaunchCheck.Checked;
  SyncRunFromLaunchCheck();
end;

procedure LaunchCheckClick(Sender: TObject);
begin
  SyncRunFromLaunchCheck();
end;

procedure InitializeWizard();
var
  Pad: Integer;
begin
  { Inner pages put labels at Left=0. At 200% DPI the first glyphs are clipped
    by the notebook edge / window radius. Keep at least the header inset, then
    add a little more. }
  Pad := WizardForm.InnerNotebook.Left;
  if WizardForm.PageNameLabel.Left > Pad then
    Pad := WizardForm.PageNameLabel.Left;
  Pad := Pad + ScaleX(12);
  WizardForm.InnerNotebook.Width :=
    WizardForm.InnerNotebook.Width - (Pad - WizardForm.InnerNotebook.Left);
  WizardForm.InnerNotebook.Left := Pad;

  { TNewCheckListBox's themed checkbox is wider than its text indent at 200%
    DPI, covering the start of "Create" / "Launch". Keep the native lists for
    state but paint a box and caption that cannot overlap. }
  HideNativeCheckList(WizardForm.TasksList);

  TasksGroupLabel := TNewStaticText.Create(WizardForm);
  TasksGroupLabel.Parent := WizardForm.SelectTasksPage;
  TasksGroupLabel.Left := WizardForm.SelectTasksLabel.Left;
  TasksGroupLabel.Top :=
    WizardForm.SelectTasksLabel.Top + WizardForm.SelectTasksLabel.Height + ScaleY(16);
  TasksGroupLabel.AutoSize := True;
  TasksGroupLabel.Caption := 'Additional shortcuts:';

  DesktopCheck := TNewCheckBox.Create(WizardForm);
  DesktopCheck.Parent := WizardForm.SelectTasksPage;
  DesktopCheck.Left := WizardForm.SelectTasksLabel.Left;
  DesktopCheck.Top :=
    TasksGroupLabel.Top + TasksGroupLabel.Height + ScaleY(8);
  DesktopCheck.Width := ScaleX(20);
  DesktopCheck.Height := ScaleY(20);
  DesktopCheck.Caption := '';
  DesktopCheck.Checked := False;
  DesktopCheck.OnClick := @DesktopCheckClick;

  DesktopCheckLabel := TNewStaticText.Create(WizardForm);
  DesktopCheckLabel.Parent := WizardForm.SelectTasksPage;
  DesktopCheckLabel.Left := DesktopCheck.Left + DesktopCheck.Width + ScaleX(8);
  DesktopCheckLabel.Top := DesktopCheck.Top + ScaleY(2);
  DesktopCheckLabel.AutoSize := True;
  DesktopCheckLabel.Caption := 'Create a desktop shortcut';
  DesktopCheckLabel.OnClick := @DesktopCheckLabelClick;

  LaunchCheck := TNewCheckBox.Create(WizardForm);
  LaunchCheck.Parent := WizardForm.FinishedPage;
  LaunchCheck.Left := WizardForm.RunList.Left;
  LaunchCheck.Top := WizardForm.RunList.Top;
  LaunchCheck.Width := ScaleX(20);
  LaunchCheck.Height := ScaleY(20);
  LaunchCheck.Caption := '';
  LaunchCheck.Checked := True;
  LaunchCheck.OnClick := @LaunchCheckClick;

  LaunchCheckLabel := TNewStaticText.Create(WizardForm);
  LaunchCheckLabel.Parent := WizardForm.FinishedPage;
  LaunchCheckLabel.Left := LaunchCheck.Left + LaunchCheck.Width + ScaleX(8);
  LaunchCheckLabel.Top := LaunchCheck.Top + ScaleY(2);
  LaunchCheckLabel.AutoSize := True;
  LaunchCheckLabel.Caption := 'Launch LocalTeX';
  LaunchCheckLabel.OnClick := @LaunchCheckLabelClick;

  HideNativeCheckList(WizardForm.RunList);
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = wpSelectTasks then
  begin
    HideNativeCheckList(WizardForm.TasksList);
    SyncDesktopCheckFromTasks();
  end;
  if CurPageID = wpFinished then
  begin
    { Capture the restored native list position before collapsing it. }
    if WizardForm.RunList.Height > 0 then
    begin
      LaunchCheck.Left := WizardForm.RunList.Left;
      LaunchCheck.Top := WizardForm.RunList.Top;
      LaunchCheckLabel.Left := LaunchCheck.Left + LaunchCheck.Width + ScaleX(8);
      LaunchCheckLabel.Top := LaunchCheck.Top + ScaleY(2);
    end;
    HideNativeCheckList(WizardForm.RunList);
    if WizardForm.RunList.Items.Count > 0 then
    begin
      LaunchCheck.Visible := True;
      LaunchCheckLabel.Visible := True;
      LaunchCheckLabel.Caption := WizardForm.RunList.ItemCaption[0];
      SyncLaunchCheckFromRun();
    end
    else
    begin
      LaunchCheck.Visible := False;
      LaunchCheckLabel.Visible := False;
    end;
  end;
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if CurPageID = wpSelectTasks then
    SyncTasksFromDesktopCheck();
  if CurPageID = wpFinished then
    SyncRunFromLaunchCheck();
end;

function InitializeUninstall(): Boolean;
begin
  Result := True;
  DeleteUserData := False;
  if not UninstallSilent then
  begin
    if MsgBox(
         'Do you also want to delete your LocalTeX settings and snip library?'#13#10#13#10
         'Choose No to keep them for a future install.',
         mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      DeleteUserData := True;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and DeleteUserData then
  begin
    DelTree(ExpandConstant('{userappdata}\localtex'), True, True, True);
    DelTree(ExpandConstant('{localappdata}\localtex'), True, True, True);
  end;
end;
