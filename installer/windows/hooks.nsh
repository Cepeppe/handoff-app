; Baton's part of the Windows installer (TECHNICAL-DESIGN §3.5, FM-24, SRV-25).
;
; Tauri generates the whole NSIS script from its own template and inserts these macros at
; four fixed places (`bundle.windows.nsis.installerHooks` in `src-tauri/tauri.conf.json`).
; Everything the template already does — a per-user install into `%LOCALAPPDATA%\Baton\`,
; the shortcuts, the uninstall key, the `Run` login entry, the "Delete the application data"
; box and its two `com.cepeppe.baton` folders — is left to it. This file adds only what the
; template cannot know about Baton:
;
;   NSIS_HOOK_PREINSTALL     an agent session may be running the bundled server: move it
;                            out of the way instead of failing to overwrite it (FM-24)
;   NSIS_HOOK_PREUNINSTALL   the superseded servers the app's launch cleanup never reached
;   NSIS_HOOK_POSTUNINSTALL  the login approval marker, and Baton's own data folder when the
;                            user ticked "Delete the application data"
;
; What it never touches, in any case: `%USERPROFILE%\.handoff\` (the channel token and the
; runbooks, shared with the server and kept through an uninstall, RUN-03) and every agent
; configuration. Those entries are removed from inside Baton (Settings → Agents → Remove),
; never by the uninstaller.
;
; The macros run inside the template's sections, so they see its defines (`PRODUCTNAME`,
; `UNINSTKEY`) and its variables (`$UpdateMode`, `$DeleteAppDataCheckboxState`). The hook
; test (`installer/windows/test/hooks-test.nsi`) declares the same names and drives each
; macro against a temporary folder; the two `!ifndef` defines below exist so that it can
; point them at places of its own. No build of the product defines them.

; The template includes it already; the guard inside makes a second include free, and the
; hook test needs nothing else.
!include LogicLib.nsh

; The bundled server as the installer writes it: `externalBin` in `tauri.conf.json` is
; `binaries/handoff-mcp`, and Tauri installs it beside the application as `handoff-mcp.exe`,
; which is the fixed path every agent configuration names (SRV-25).
!define BATON_SERVER "handoff-mcp"

; The approval marker Windows keeps next to a `Run` login entry. `tauri-plugin-autostart`
; writes both; the template deletes the `Run` value and not this one (T-041).
!ifndef BATON_STARTUP_APPROVED_KEY
  !define BATON_STARTUP_APPROVED_KEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
!endif

; FM-24: "An executable in use cannot be overwritten but can be renamed" (§3.5).
;
; The installed server is the file an agent runs for as long as its session lasts, so an
; update started while any session is open meets it locked, and the template's plain `File`
; would stop the update with "Error opening file for writing". Deleting first is the
; question "is anybody running it?" asked the only reliable way, by trying: a file nobody
; holds goes, and the update then writes the new one as if it were a first install. A file
; a session is running refuses to be deleted and can still be renamed, so it becomes
; `handoff-mcp.<old version>.old.exe`: the running session keeps the binary it opened, the
; new server lands at the same path, and every agent configuration stays valid. The app
; deletes the renamed file at its next launch once nobody holds it
; (`src-tauri/src/install/cleanup.rs`), and that name is what it looks for.
;
; The old version is the one the uninstall key still records, since the template writes the
; new one only after the files are copied. A name already taken by an earlier update whose
; session is still running gets a counter instead of being reused.
!macro NSIS_HOOK_PREINSTALL
  Push $0
  Push $1
  Push $2
  ${If} ${FileExists} "$INSTDIR\${BATON_SERVER}.exe"
    ClearErrors
    Delete "$INSTDIR\${BATON_SERVER}.exe"
    ${If} ${Errors}
      ReadRegStr $0 SHCTX "${UNINSTKEY}" "DisplayVersion"
      ${If} $0 == ""
        StrCpy $0 "unknown"
      ${EndIf}
      StrCpy $1 "$INSTDIR\${BATON_SERVER}.$0.old.exe"
      StrCpy $2 0
      ${DoWhile} ${FileExists} "$1"
        ClearErrors
        Delete "$1"
        ${IfNot} ${Errors}
          ${Break}
        ${EndIf}
        IntOp $2 $2 + 1
        ${If} $2 > 99
          ${Break}
        ${EndIf}
        StrCpy $1 "$INSTDIR\${BATON_SERVER}.$0-$2.old.exe"
      ${Loop}
      ClearErrors
      Rename "$INSTDIR\${BATON_SERVER}.exe" "$1"
      ${If} ${Errors}
        DetailPrint "The running ${BATON_SERVER}.exe could not be moved aside; close your agent sessions and run the setup again."
      ${Else}
        DetailPrint "A running ${BATON_SERVER}.exe was moved aside to $1; agent sessions keep it until they end."
      ${EndIf}
    ${EndIf}
  ${EndIf}
  ClearErrors
  Pop $2
  Pop $1
  Pop $0
!macroend

; Before the template removes the program folder: the superseded servers FM-24 left behind.
; Once the application is gone nobody else will delete them, and a folder that still holds
; one cannot be removed. One a session is still running is left where it is — a delete
; cannot take a file a process has open — along with the folder around it.
!macro NSIS_HOOK_PREUNINSTALL
  Delete "$INSTDIR\${BATON_SERVER}.*.old.exe"
  ClearErrors
!macroend

; After the template's own cleanup. Neither step runs when a newer setup is replacing this
; one (`/UPDATE`), because the installation is about to continue.
;
; - The login approval marker, which the template leaves behind (T-041): a marker for an
;   entry that is gone is harmless, but it is Baton's, and the uninstall is when it goes.
; - `%APPDATA%\Baton\`, only when the user ticked "Delete the application data" (unticked
;   by default, and never in a silent uninstall). The template's box removes Tauri's two
;   bundle-identifier folders, `%APPDATA%\com.cepeppe.baton` and `%LOCALAPPDATA%\
;   com.cepeppe.baton`, which hold the webview's data; Baton's database and crash files are
;   in `%APPDATA%\Baton\` (`paths::app_data_dir`, §7.2), and the box promises "the
;   application data", so it removes that folder too (owner's decision, 2026-09-10).
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "${BATON_STARTUP_APPROVED_KEY}" "${PRODUCTNAME}"
  ${EndIf}
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    !ifdef BATON_APP_DATA_DIR
      RMDir /r "${BATON_APP_DATA_DIR}"
    !else
      SetShellVarContext current
      RMDir /r "$APPDATA\${PRODUCTNAME}"
    !endif
  ${EndIf}
  ClearErrors
!macroend
