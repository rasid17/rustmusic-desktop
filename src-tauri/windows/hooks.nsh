; Ensure RustMusic closes gracefully first (flushing SQLite WAL and restoring audio state),
; then force-terminates lingering child processes only if still running during upgrade or uninstall.
!macro NSIS_HOOK_PREINSTALL
  nsExec::ExecToLog 'taskkill /IM "RustMusic.exe"'
  Sleep 1200
  nsExec::ExecToLog 'taskkill /F /T /IM "RustMusic.exe"'
  Sleep 500
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog 'taskkill /IM "RustMusic.exe"'
  Sleep 1200
  nsExec::ExecToLog 'taskkill /F /T /IM "RustMusic.exe"'
  Sleep 500
!macroend
