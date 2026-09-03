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
; Inno 6+ defaults to 120,120 for Setup and Uninstall.
WizardSizePercent=100,100
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
  UninstallWipeCheck: TNewCheckBox;

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
  LaunchCheck.Width := ScaleX(20);
  LaunchCheck.Height := ScaleY(20);
  LaunchCheck.Caption := '';
  LaunchCheck.Checked := True;
  LaunchCheck.Visible := False;
  LaunchCheck.OnClick := @LaunchCheckClick;

  LaunchCheckLabel := TNewStaticText.Create(WizardForm);
  LaunchCheckLabel.Parent := WizardForm.FinishedPage;
  LaunchCheckLabel.AutoSize := True;
  LaunchCheckLabel.Caption := 'Launch LocalTeX';
  LaunchCheckLabel.Visible := False;
  LaunchCheckLabel.OnClick := @LaunchCheckLabelClick;
end;

procedure PlaceLaunchCheck();
begin
  { RunList.Top is the DFM slot until wpFinished. HideNativeCheckList also
    zeroes Height before Inno lays out ClickFinish, so that slot sits on the
    last FinishedLabel line. Anchor under the label instead. }
  WizardForm.FinishedLabel.Width :=
    WizardForm.FinishedPage.ClientWidth - WizardForm.FinishedLabel.Left;
  WizardForm.AdjustLabelHeight(WizardForm.FinishedLabel);
  LaunchCheck.Left := WizardForm.FinishedLabel.Left;
  LaunchCheck.Top :=
    WizardForm.FinishedLabel.Top + WizardForm.FinishedLabel.Height + ScaleY(8);
  LaunchCheckLabel.Left := LaunchCheck.Left + LaunchCheck.Width + ScaleX(8);
  LaunchCheckLabel.Top := LaunchCheck.Top + ScaleY(2);
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
    HideNativeCheckList(WizardForm.RunList);
    if WizardForm.RunList.Items.Count > 0 then
    begin
      PlaceLaunchCheck();
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

procedure PlaceSeparatedCheck(
  Owner: TComponent; Parent: TWinControl;
  var Box: TNewCheckBox; var Caption: TNewStaticText;
  Left, Top: Integer; const Text: String; DefaultChecked: Boolean);
begin
  Box := TNewCheckBox.Create(Owner);
  Box.Parent := Parent;
  Box.Left := Left;
  Box.Top := Top;
  Box.Width := ScaleX(20);
  Box.Height := ScaleY(20);
  Box.Caption := '';
  Box.Checked := DefaultChecked;

  Caption := TNewStaticText.Create(Owner);
  Caption.Parent := Parent;
  Caption.Left := Box.Left + Box.Width + ScaleX(8);
  Caption.Top := Box.Top + ScaleY(2);
  Caption.AutoSize := True;
  Caption.Caption := Text;
end;

procedure UninstallWipeLabelClick(Sender: TObject);
begin
  UninstallWipeCheck.Checked := not UninstallWipeCheck.Checked;
end;

function InitializeUninstall(): Boolean;
begin
  Result := True;
  DeleteUserData := False;
end;

procedure InitializeUninstallProgressForm();
var
  WipePage: TNewNotebookPage;
  WipeLabel: TNewStaticText;
  UninstallButton: TNewButton;
  SavedName, SavedDescription: String;
  SavedCancelEnabled: Boolean;
  SavedCancelModal: Integer;
begin
  { Inno already showed Confirm Uninstall. Offer wipe on the wizard, not
    as a modal that runs before that question. Silent keeps user data. }
  if UninstallSilent then
    Exit;

  WipePage := TNewNotebookPage.Create(UninstallProgressForm);
  WipePage.Notebook := UninstallProgressForm.InnerNotebook;
  WipePage.Parent := UninstallProgressForm.InnerNotebook;
  WipePage.Align := alClient;

  PlaceSeparatedCheck(
    UninstallProgressForm, WipePage, UninstallWipeCheck, WipeLabel,
    UninstallProgressForm.StatusLabel.Left,
    UninstallProgressForm.StatusLabel.Top,
    'Also delete my LocalTeX settings and snip library', False);
  WipeLabel.OnClick := @UninstallWipeLabelClick;

  UninstallButton := TNewButton.Create(UninstallProgressForm);
  UninstallButton.Parent := UninstallProgressForm;
  UninstallButton.Width := UninstallProgressForm.CancelButton.Width;
  UninstallButton.Height := UninstallProgressForm.CancelButton.Height;
  UninstallButton.Left :=
    UninstallProgressForm.CancelButton.Left - UninstallButton.Width - ScaleX(10);
  UninstallButton.Top := UninstallProgressForm.CancelButton.Top;
  UninstallButton.Caption := 'Uninstall';
  UninstallButton.ModalResult := mrOk;
  UninstallButton.Default := True;

  UninstallProgressForm.InnerNotebook.ActivePage := WipePage;
  SavedName := UninstallProgressForm.PageNameLabel.Caption;
  SavedDescription := UninstallProgressForm.PageDescriptionLabel.Caption;
  UninstallProgressForm.PageNameLabel.Caption := 'User data';
  UninstallProgressForm.PageDescriptionLabel.Caption :=
    'Settings and the snip library stay unless you check the box below.';

  SavedCancelEnabled := UninstallProgressForm.CancelButton.Enabled;
  SavedCancelModal := UninstallProgressForm.CancelButton.ModalResult;
  UninstallProgressForm.CancelButton.Enabled := True;
  UninstallProgressForm.CancelButton.ModalResult := mrCancel;
  UninstallProgressForm.ActiveControl := UninstallButton;

  if UninstallProgressForm.ShowModal() = mrCancel then
    Abort;

  DeleteUserData := UninstallWipeCheck.Checked;
  UninstallButton.Visible := False;
  UninstallProgressForm.CancelButton.Enabled := SavedCancelEnabled;
  UninstallProgressForm.CancelButton.ModalResult := SavedCancelModal;
  UninstallProgressForm.PageNameLabel.Caption := SavedName;
  UninstallProgressForm.PageDescriptionLabel.Caption := SavedDescription;
  UninstallProgressForm.InnerNotebook.ActivePage :=
    UninstallProgressForm.InstallingPage;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and DeleteUserData then
  begin
    Log('Deleting LocalTeX user data');
    DelTree(ExpandConstant('{userappdata}\localtex'), True, True, True);
    DelTree(ExpandConstant('{localappdata}\localtex'), True, True, True);
  end;
end;
