; EEW View Windows installer (NSIS)
Unicode true
RequestExecutionLevel admin

!ifndef VERSION
  !define VERSION "1.4.0"
!endif
!ifndef DIST_DIR
  !define DIST_DIR "..\dist\EEWView"
!endif
!ifndef OUT_FILE
  !define OUT_FILE "..\dist\EEWView_Beta_${VERSION}_setup.exe"
!endif
!ifndef SPLASH_BMP
  !define SPLASH_BMP "${DIST_DIR}\assets\brand\splash_installer.bmp"
!endif

Name "EEW View"
OutFile "${OUT_FILE}"
InstallDir "$PROGRAMFILES32\eew view"
InstallDirRegKey HKLM "Software\eew view" "InstallDir"
SetCompressor /SOLID lzma

!include "MUI2.nsh"

!define MUI_ABORTWARNING
!define MUI_ICON "${DIST_DIR}\assets\brand\app.ico"
!define MUI_UNICON "${DIST_DIR}\assets\brand\app.ico"

; Welcome/finish sidebar bitmap (generated from splash_bg.png at pack time)
!define MUI_WELCOMEFINISHPAGE_BITMAP "${SPLASH_BMP}"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "${SPLASH_BMP}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

Section "EEW View" SecMain
  SetOutPath "$INSTDIR"
  File /r "${DIST_DIR}\*.*"

  WriteRegStr HKLM "Software\eew view" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EEWView" \
    "DisplayName" "EEW View"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EEWView" \
    "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EEWView" \
    "Publisher" "Jian11323"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EEWView" \
    "DisplayIcon" "$INSTDIR\eewview.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EEWView" \
    "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EEWView" \
    "NoModify" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EEWView" \
    "NoRepair" 1

  CreateDirectory "$SMPROGRAMS\EEW View"
  CreateShortCut "$SMPROGRAMS\EEW View\EEW View.lnk" "$INSTDIR\eewview.exe" "" "$INSTDIR\assets\brand\app.ico" 0 SW_SHOWNORMAL "" "$INSTDIR"
  CreateShortCut "$DESKTOP\EEW View.lnk" "$INSTDIR\eewview.exe" "" "$INSTDIR\assets\brand\app.ico" 0 SW_SHOWNORMAL "" "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
SectionEnd

Section "Uninstall"
  Delete "$DESKTOP\EEW View.lnk"
  Delete "$SMPROGRAMS\EEW View\EEW View.lnk"
  RMDir "$SMPROGRAMS\EEW View"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\EEWView"
  DeleteRegKey HKLM "Software\eew view"
  RMDir /r "$INSTDIR"
SectionEnd
