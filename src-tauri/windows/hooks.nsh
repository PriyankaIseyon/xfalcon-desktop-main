; NSIS hooks for the xFalcon Windows installer. Referenced from
; bundle.windows.nsis.installerHooks in tauri.conf.json.

!macro NSIS_HOOK_PREUNINSTALL
  ; An uninstall does what Settings > Data & reset > Reset xFalcon does: it runs
  ; the app's own purge, which removes the app's data, caches, logs, WebView
  ; storage and the credential-store entries.
  ;
  ; The purge must NOT run when this uninstaller is only a step of an upgrade:
  ;
  ; - The silent updater runs it with /UPDATE ($UpdateMode = 1).
  ; - Running a newer setup over an older install ("Uninstall before
  ;   installing", the default) also runs the old uninstaller, WITHOUT /UPDATE.
  ;   The installer starts it in place with `_?=<install dir>`, so it runs from
  ;   the install directory itself. A real uninstall (Apps & features, or
  ;   running uninstall.exe) is different: NSIS first copies the uninstaller to
  ;   a temp directory and runs that copy, so $EXEDIR is the temp directory and
  ;   differs from $INSTDIR. That difference is the signal used below. Checked
  ;   with a throwaway NSIS installer on Windows: a normal uninstall logged
  ;   $EXEDIR = %TEMP%\~nsu*.tmp, an installer-style `_?=` run logged
  ;   $EXEDIR = $INSTDIR.
  ;
  ; A person who ticks "Delete the application data" during an in-place run
  ; asked for the data to go, so the checkbox also triggers the purge.
  ;
  ; Known limit: a scripted `uninstall.exe _?=<dir>` run is indistinguishable
  ; from an upgrade step, so it does not purge.
  ${If} $UpdateMode <> 1
    ${If} $EXEDIR != $INSTDIR
    ${OrIf} $DeleteAppDataCheckboxState = 1
      ; The app must not hold its files while the purge deletes them.
      !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

      ; --retry-until-clean repeats the purge while files remain. WebView2
      ; processes (msedgewebview2) can exit a moment after the app does, and
      ; they hold files until they do.
      nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --purge-local-data --retry-until-clean'
      Pop $0
      ; A leftover is reported in the uninstall log only. It must never stop
      ; the uninstall, because the program files still have to be removed.
      ${If} $0 != 0
        DetailPrint "xFalcon: some local data could not be removed (exit code $0)"
      ${EndIf}
    ${EndIf}
  ${EndIf}
!macroend
