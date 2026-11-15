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
