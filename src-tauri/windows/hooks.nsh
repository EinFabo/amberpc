; Amber's additions to the Tauri NSIS installer.

; Uninstalling removes the autostart entry the app wrote (HKCU Run, value "Amber") and
; Task Manager's switch for it, so Windows does not try to start a program that is gone.
; Not on updates: the updater runs the installer with /UPDATE and skips the uninstall.
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Amber"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Amber"
!macroend
