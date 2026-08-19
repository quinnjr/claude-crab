; SPDX-License-Identifier: MIT
;
; Inno Setup script for the Windows installer. Compile from the repository
; root after a release build:
;
;   ISCC.exe /DMyAppVersion=2.0.1 /Odist packaging\windows\claude-crab.iss

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif

[Setup]
AppId={{dev.quinnjr.claude-crab}
AppName=Claude Crab
AppVersion={#MyAppVersion}
AppPublisher=Joseph R. Quinn
AppPublisherURL=https://github.com/quinnjr/claude-crab
DefaultDirName={autopf}\Claude Crab
DefaultGroupName=Claude Crab
DisableProgramGroupPage=yes
LicenseFile=..\..\LICENSE
OutputBaseFilename=claude-crab-{#MyAppVersion}-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Files]
Source: "..\..\target\release\claude-crab.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\tools\crab_hooks.py"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Claude Crab"; Filename: "{app}\claude-crab.exe"

[Tasks]
Name: "startup"; Description: "Start Claude Crab when you sign in"; Flags: unchecked

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; \
  ValueType: string; ValueName: "Claude Crab"; \
  ValueData: """{app}\claude-crab.exe"""; Tasks: startup; Flags: uninsdeletevalue
