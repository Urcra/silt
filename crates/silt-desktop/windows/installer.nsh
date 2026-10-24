; Puts the install directory, which holds silt.exe beside the app, on the
; user's PATH, and takes it off again (ADR 0045). The directory reaches
; PowerShell through the environment, so no quote in it can break the line.

!macro NSIS_HOOK_POSTINSTALL
  System::Call 'Kernel32::SetEnvironmentVariable(t "SILT_INSTALL_DIR", t "$INSTDIR")i'
  nsExec::Exec `powershell.exe -NoProfile -NonInteractive -Command "$$dir = $$env:SILT_INSTALL_DIR; $$path = [Environment]::GetEnvironmentVariable('Path', 'User'); if (-not (($$path -split ';') -contains $$dir)) { if ($$path) { $$path = $$path.TrimEnd(';') + ';' }; [Environment]::SetEnvironmentVariable('Path', $$path + $$dir, 'User') }"`
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  System::Call 'Kernel32::SetEnvironmentVariable(t "SILT_INSTALL_DIR", t "$INSTDIR")i'
  nsExec::Exec `powershell.exe -NoProfile -NonInteractive -Command "$$dir = $$env:SILT_INSTALL_DIR; $$path = [Environment]::GetEnvironmentVariable('Path', 'User'); if ($$path) { [Environment]::SetEnvironmentVariable('Path', ((($$path -split ';') | Where-Object { $$_ -and $$_ -ne $$dir }) -join ';'), 'User') }"`
!macroend
