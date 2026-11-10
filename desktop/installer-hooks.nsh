; Versions up to 0.2.6 installed the app as "Superteem Voice". From 0.3 it is "Voice by Superteem", which
; Windows treats as another app, so the old copy is removed first: one app in the Start menu, one in
; Installed apps. Recordings, notes and settings are kept: they don't live in the install folder.
!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Superteem Voice" "InstallLocation"
  ${If} $R0 != ""
    ; Tauri writes the location in quotes.
    StrCpy $R1 $R0 1
    ${If} $R1 == '"'
      StrCpy $R0 $R0 "" 1
      StrCpy $R0 $R0 -1
    ${EndIf}
    ${If} ${FileExists} "$R0\uninstall.exe"
      DetailPrint "Removing the earlier Superteem Voice from $R0"
      !insertmacro CheckIfAppIsRunning "$R0\superteem-voice.exe" "Superteem Voice"
      ; _?= runs the uninstaller in place, so this waits for it to finish.
      ExecWait '"$R0\uninstall.exe" /S _?=$R0'
      Delete "$R0\uninstall.exe"
      RMDir "$R0"
    ${EndIf}
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Superteem Voice"
    DeleteRegKey HKCU "Software\Superteem\Superteem Voice"
  ${EndIf}
!macroend

; Natural reading voices run on sherpa-onnx, whose libraries the build puts beside the app. They're
; installed with it; the app loads them only when the voice is used, so it starts without them.
!macro STV_NATIVE_FILE NAME
  ; STV_NATIVE_DIR is the build's output folder, set by the release build (Tauri compiles a copy of this
  ; file elsewhere, so a path relative to it doesn't work). A build that sets it must have the file.
  !if "$%STV_NATIVE_DIR%" != "${U+24}%STV_NATIVE_DIR%"
    !if /FileExists "$%STV_NATIVE_DIR%\${NAME}"
      File "$%STV_NATIVE_DIR%\${NAME}"
    !else
      !error "${NAME} isn't in $%STV_NATIVE_DIR%"
    !endif
  !endif
!macroend

!macro NSIS_HOOK_POSTINSTALL
  SetOutPath $INSTDIR
  !insertmacro STV_NATIVE_FILE "sherpa-onnx-c-api.dll"
  !insertmacro STV_NATIVE_FILE "onnxruntime.dll"
  !insertmacro STV_NATIVE_FILE "onnxruntime_providers_shared.dll"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\sherpa-onnx-c-api.dll"
  Delete "$INSTDIR\onnxruntime.dll"
  Delete "$INSTDIR\onnxruntime_providers_shared.dll"
!macroend
