!include "LogicLib.nsh"

!macro NSIS_HOOK_PREUNINSTALL
  ; Keep login registration during install-over updates. Remove only an entry
  ; pointing at this installed copy; a standalone copy may own the same name.
  ${If} $UpdateMode <> 1
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "RusTor"
    ${If} $0 == '$\"$INSTDIR\rustor-desktop.exe$\" --autostart'
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "RusTor"
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "RusTor"
    ${EndIf}
  ${EndIf}
!macroend
