; Amber's additions to the Tauri NSIS installer.

; Uninstalling removes the autostart entry the app wrote (HKCU Run, value "Amber"), so
; Windows does not try to start a program that is gone.
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Amber"
!macroend
