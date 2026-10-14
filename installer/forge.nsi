; Forge — Windows setup wizard (NSIS).  Build: makensis -DVERSION=0.1.0 installer/forge.nsi
Unicode true
!include "MUI2.nsh"
!include "nsDialogs.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!ifndef VERSION
  !define VERSION "0.1.0"
!endif
!ifndef EXE
  !define EXE "..\target\x86_64-pc-windows-gnu\release\forge.exe"
!endif

Name "Forge ${VERSION}"
OutFile "..\dist\Forge-Setup.exe"
InstallDir "$LOCALAPPDATA\Programs\Forge"
InstallDirRegKey HKCU "Software\Forge" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "Forge · made by burkiuze"

!define MUI_ICON "..\assets\forge.ico"
!define MUI_UNICON "..\assets\forge.ico"
!define MUI_WELCOMEFINISHPAGE_BITMAP "..\assets\installer-side.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "..\assets\installer-side.bmp"
!define MUI_ABORTWARNING

Var KeyDialog
Var KeyField
Var ApiKey

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
Page custom KeyPage KeyPageLeave
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\Forge.exe"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "Turkish"
!insertmacro MUI_LANGUAGE "English"

LangString KeyTitle ${LANG_TURKISH} "Forge AI bağlantısı"
LangString KeyTitle ${LANG_ENGLISH} "Forge AI connection"
LangString KeySub ${LANG_TURKISH} "OpenRouter API anahtarınızı girin (isteğe bağlı)"
LangString KeySub ${LANG_ENGLISH} "Enter your OpenRouter API key (optional)"
LangString KeyBody ${LANG_TURKISH} "Anahtar yalnızca bu bilgisayarda %APPDATA%\Forge\.env dosyasına kaydedilir. Boş bırakırsanız Forge ilk açılışta kurulum sihirbazında soracaktır."
LangString KeyBody ${LANG_ENGLISH} "The key is stored only on this PC in %APPDATA%\Forge\.env. Leave it empty and Forge will ask in its first-run wizard."

Function .onInit
  ; Silent mode = self-update from inside Forge: give the old process time to exit.
  IfSilent 0 +3
    Sleep 2500
    Goto done
  !insertmacro MUI_LANGDLL_DISPLAY
  done:
FunctionEnd

Function .onInstSuccess
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/RUN" $R1
  IfErrors +2
    Exec '"$INSTDIR\Forge.exe"'
FunctionEnd

Function KeyPage
  !insertmacro MUI_HEADER_TEXT "$(KeyTitle)" "$(KeySub)"
  nsDialogs::Create 1018
  Pop $KeyDialog
  ${If} $KeyDialog == error
    Abort
  ${EndIf}
  ${NSD_CreateLabel} 0 0 100% 36u "$(KeyBody)"
  Pop $0
  ${NSD_CreateLabel} 0 44u 100% 10u "OPENROUTER_API_KEY"
  Pop $0
  ${NSD_CreatePassword} 0 56u 100% 13u "$ApiKey"
  Pop $KeyField
  ${NSD_CreateLink} 0 76u 100% 10u "https://openrouter.ai/keys"
  Pop $0
  ${NSD_OnClick} $0 OpenKeysLink
  nsDialogs::Show
FunctionEnd

Function OpenKeysLink
  ExecShell "open" "https://openrouter.ai/keys"
FunctionEnd

Function KeyPageLeave
  ${NSD_GetText} $KeyField $ApiKey
FunctionEnd

Section "Forge (gerekli / required)" SecCore
  SectionIn RO
  SetOutPath "$INSTDIR"
  SetOverwrite try
  ClearErrors
  File /oname=Forge.exe "${EXE}"
  IfErrors 0 +4
    Sleep 3000
    ClearErrors
    File /oname=Forge.exe "${EXE}"
  SetOverwrite on
  File "..\assets\forge.ico"
  File /oname=LICENSE.txt "..\LICENSE"
  File /oname=README.md "..\README.md"
  ${If} $ApiKey != ""
    CreateDirectory "$APPDATA\Forge"
    FileOpen $0 "$APPDATA\Forge\.env" a
    FileSeek $0 0 END
    FileWrite $0 "$\r$\nOPENROUTER_API_KEY=$ApiKey$\r$\n"
    FileClose $0
  ${EndIf}
  WriteRegStr HKCU "Software\Forge" "InstallDir" "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  !define UNKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Forge"
  WriteRegStr HKCU "${UNKEY}" "DisplayName" "Forge"
  WriteRegStr HKCU "${UNKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNKEY}" "Publisher" "Forge"
  WriteRegStr HKCU "${UNKEY}" "DisplayIcon" "$INSTDIR\forge.ico"
  WriteRegStr HKCU "${UNKEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNKEY}" "NoRepair" 1
SectionEnd

Section "Başlat menüsü / Start Menu" SecStart
  CreateDirectory "$SMPROGRAMS\Forge"
  CreateShortcut "$SMPROGRAMS\Forge\Forge.lnk" "$INSTDIR\Forge.exe" "" "$INSTDIR\forge.ico"
  CreateShortcut "$SMPROGRAMS\Forge\Uninstall Forge.lnk" "$INSTDIR\Uninstall.exe"
SectionEnd

Section "Masaüstü kısayolu / Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\Forge.lnk" "$INSTDIR\Forge.exe" "" "$INSTDIR\forge.ico"
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\Forge.exe"
  Delete "$INSTDIR\forge.ico"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Forge\Forge.lnk"
  Delete "$SMPROGRAMS\Forge\Uninstall Forge.lnk"
  RMDir "$SMPROGRAMS\Forge"
  Delete "$DESKTOP\Forge.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Forge"
  DeleteRegKey HKCU "Software\Forge"
  ; User data (%APPDATA%\Forge: settings, .env, downloaded models) is kept on purpose.
SectionEnd
