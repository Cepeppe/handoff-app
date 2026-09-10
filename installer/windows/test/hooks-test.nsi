; Drives the macros of `installer/windows/hooks.nsh` outside Tauri's installer: one macro per
; run, against the folder the command line names (`src/__tests__/installer-hooks.test.ts`).
;
; Tauri's template inserts the hooks into its own sections, where a few of its names are in
; scope. This script declares the same names and nothing else, so a hook that reached for
; anything more would fail to compile here:
;
;   PRODUCTNAME, UNINSTKEY          defines, given on the command line of makensis
;   $UpdateMode                     1 when a newer setup is replacing this one (`/UPDATE`)
;   $DeleteAppDataCheckboxState     1 when "Delete the application data" was ticked
;
; The test also defines BATON_APP_DATA_DIR and BATON_STARTUP_APPROVED_KEY, so that nothing a
; hook deletes can be a real installation's.
;
; Usage, once built:
;
;   hooks-test.exe /STEP=preinstall|preuninstall|postuninstall [/UPDATE] [/DELETEAPPDATA] /D=<folder>
;
; `/D=` is NSIS's own way of setting $INSTDIR, and it has to come last. An unknown step exits
; with 2, so a test that misspelt one fails instead of passing on a hook that never ran.

Unicode true

!include LogicLib.nsh
!include FileFunc.nsh

!ifndef OUTFILE
  !error "pass /DOUTFILE=<path of the test executable>"
!endif
!ifndef HOOKS
  !error "pass /DHOOKS=<path of installer/windows/hooks.nsh>"
!endif
!ifndef PRODUCTNAME
  !error "pass /DPRODUCTNAME=<a throwaway product name>"
!endif
!ifndef UNINSTKEY
  !error "pass /DUNINSTKEY=<a throwaway key under HKCU>"
!endif

Name "${PRODUCTNAME}"
OutFile "${OUTFILE}"
RequestExecutionLevel user
SilentInstall silent
InstallDir "$TEMP\${PRODUCTNAME}"

Var UpdateMode
Var DeleteAppDataCheckboxState

!include "${HOOKS}"

Section
  ${GetOptions} $CMDLINE "/UPDATE" $0
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
  ${GetOptions} $CMDLINE "/DELETEAPPDATA" $0
  ${IfNot} ${Errors}
    StrCpy $DeleteAppDataCheckboxState 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/STEP=" $0
  ${If} $0 == "preinstall"
    !insertmacro NSIS_HOOK_PREINSTALL
  ${ElseIf} $0 == "preuninstall"
    !insertmacro NSIS_HOOK_PREUNINSTALL
  ${ElseIf} $0 == "postuninstall"
    !insertmacro NSIS_HOOK_POSTUNINSTALL
  ${Else}
    SetErrorLevel 2
  ${EndIf}
SectionEnd
