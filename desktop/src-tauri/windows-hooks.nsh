; Windows-only installer hooks for the "Reader" -> "Reader-L" rename.
;
; Older releases shipped under the product name "Reader", which collides with
; Readwise Reader: both wrote their Add/Remove Programs entry to
; `...\Uninstall\Reader`, which is what made WinGet treat them as one package.
; This installer ships as "Reader-L", so it has to adopt what the previous name
; left behind — otherwise an upgrade would look like a fresh install (duplicate
; entry in the installed-apps list) and the library would come up empty.
;
; That shared name means the `Reader` keys cannot be assumed to be ours, so two
; independent facts are tracked, and every deletion is guarded by the narrowest
; one that applies:
;
;   $R3 = 1  the Add/Remove Programs key is ours (`Publisher` is `hadc188`)
;   $R9 = 1  we have evidence of an earlier install of ours at all, which is
;            also true when only our own `Software\hadc188\Reader` key is left
;
; `$R9` guards cleanup of things that are unambiguously ours (our publisher key,
; shortcuts verified to point into the old install directory). `$R3` guards the
; Add/Remove Programs key itself, because Readwise Reader may own it.

!include LogicLib.nsh

!define READER_OLD_UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Reader"
!define READER_OLD_MANUPRODUCTKEY "Software\hadc188\Reader"
!define READER_STABLE_MANUPRODUCTKEY "Software\hadc188\Reader"

!macro ReaderLStopOldBuild
  ; The previous build may still be running and holding the SQLite database
  ; open, which would make moving the data directory fail. `MainBinaryName` is
  ; written by the old installer ("reader-desktop.exe").
  ReadRegStr $R5 SHCTX "${READER_OLD_UNINSTKEY}" "MainBinaryName"
  ${If} $R5 == ""
    StrCpy $R5 "reader-desktop.exe"
  ${EndIf}
  nsExec::ExecToStack '"$SYSDIR\taskkill.exe" /IM "$R5" /F'
  Pop $R6
  Pop $R6
  Sleep 600
!macroend

!macro ReaderLMigrateFromOldName
  ; $R4 holds the install directory used by the previous product name. It is
  ; resolved before the old uninstaller runs, because that removes the registry
  ; entries this relies on.
  ReadRegStr $R0 SHCTX "${READER_OLD_UNINSTKEY}" "Publisher"
  ReadRegStr $R4 SHCTX "${READER_OLD_MANUPRODUCTKEY}" ""

  StrCpy $R3 0
  ${If} $R0 == "hadc188"
    StrCpy $R3 1
  ${EndIf}

  ; Our publisher key lives under `hadc188`, taken from the bundle identifier
  ; `com.hadc188.reader`, so unlike the ARP key it cannot belong to Readwise
  ; Reader. It is also what remains when the old uninstaller already removed the
  ; Add/Remove Programs entry, in which case the leftovers still need clearing.
  StrCpy $R9 0
  ${If} $R3 = 1
    StrCpy $R9 1
  ${ElseIf} $R4 != ""
    StrCpy $R9 1
  ${EndIf}
  ${If} $R9 != 1
    Goto readerl_migrate_done
  ${EndIf}

  ; `InstallLocation` is only consulted when the ARP key is ours (it is stored
  ; quoted, unlike the value read above).
  ${If} $R4 == ""
    ReadRegStr $R4 SHCTX "${READER_OLD_UNINSTKEY}" "InstallLocation"
    ${If} $R4 != ""
      StrLen $R6 $R4
      IntOp $R6 $R6 - 2
      StrCpy $R4 $R4 $R6 1
    ${EndIf}
  ${EndIf}

  ; Normalise a trailing backslash so a value equal to $INSTDIR is recognised as
  ; such: otherwise the old uninstaller below would delete the executable this
  ; installer just wrote.
  ${If} $R4 != ""
    StrCpy $R6 $R4 "" -1
    ${If} $R6 == "\"
      StrCpy $R4 $R4 -1
    ${EndIf}
  ${EndIf}

  ; Refuse a bare drive root (`C:`, `C:\`). Nothing legitimate is installed
  ; there, and a tampered registry value must not turn into deletions outside an
  ; install directory. Checked after normalising; a colon cannot appear in a
  ; real file name, so genuine paths never end in one.
  StrCpy $R6 $R4 "" -1
  ${If} $R6 == ":"
    Goto readerl_migrate_registry
  ${EndIf}

  ; Nothing left to adopt: unknown location, already our directory, or gone.
  ${If} $R4 == ""
    Goto readerl_migrate_registry
  ${EndIf}
  ${If} $R4 == "$INSTDIR"
    Goto readerl_migrate_registry
  ${EndIf}
  ${IfNot} ${FileExists} "$R4\*.*"
    Goto readerl_migrate_registry
  ${EndIf}

  !insertmacro ReaderLStopOldBuild

  ; The library lives next to the executable (portable layout), so it has to
  ; move with the install directory; otherwise the user's books, sources and
  ; reading progress would be orphaned under the old name.
  ;
  ; The inner guard means a data directory that already exists is never
  ; overwritten.
  ${If} ${FileExists} "$R4\data\*.*"
    ${IfNot} ${FileExists} "$INSTDIR\data\*.*"
      ClearErrors
      Rename "$R4\data" "$INSTDIR\data"
      ${If} ${Errors}
        ; Different volume, or still locked: fall back to a recursive move.
        ; `/R:0 /W:0` is required — robocopy defaults to a million retries with
        ; a 30 second wait, and nsExec waits for the process to exit, so one
        ; locked file would hang the installer indefinitely. This branch is
        ; reached precisely because something was locked.
        nsExec::ExecToStack '"$SYSDIR\robocopy.exe" "$R4\data" "$INSTDIR\data" /E /MOVE /R:0 /W:0 /NFL /NDL /NJH /NJS /NP'
        Pop $R7
        Pop $R6
        ; 8 and above mean at least one file could not be moved. Whatever was
        ; left behind stays in the old directory: the `RMDir`s below are
        ; non-recursive, so nothing is dropped. Warn rather than fail silently.
        ${If} $R7 >= 8
          DetailPrint "Reader-L: 部分书库文件未能移动，仍保留在 $R4\data"
          IfSilent readerl_move_failed
            MessageBox MB_OK|MB_ICONEXCLAMATION \
              "部分书库数据未能迁移到新目录。$\r$\n$\r$\n原数据仍保留在：$\r$\n$R4\data$\r$\n$\r$\n可手动移动到：$\r$\n$INSTDIR\data"
          readerl_move_failed:
        ${EndIf}
      ${EndIf}
    ${EndIf}
  ${EndIf}

  ; Let the old uninstaller remove its files, shortcuts and Add/Remove Programs
  ; entry. `_?=` keeps it from deleting itself, so its leftovers are cleared
  ; below. It must run before our publisher key is removed, because it reads
  ; `$INSTDIR` from that key.
  ${If} ${FileExists} "$R4\uninstall.exe"
    ExecWait '"$R4\uninstall.exe" /S _?=$R4' $R7
    Sleep 600
  ${EndIf}
  Delete "$R4\reader-desktop.exe"
  Delete "$R4\uninstall.exe"
  RMDir "$R4\data"
  RMDir "$R4"

  readerl_migrate_registry:
  ; The Add/Remove Programs key is removed only when it is actually ours: it may
  ; instead belong to Readwise Reader, which shares the name.
  ${If} $R3 = 1
    DeleteRegKey SHCTX "${READER_OLD_UNINSTKEY}"
  ${EndIf}
  ${If} $R9 = 1
    DeleteRegKey SHCTX "${READER_OLD_MANUPRODUCTKEY}"
  ${EndIf}

  ; Shortcuts are removed only when they still point into the old install
  ; directory, so a `Reader*.lnk` belonging to Readwise Reader is left alone.
  ; The glob also matches `Reader-L.lnk` created by this installer, whose target
  ; is the new directory and therefore never matches.
  ${If} $R9 = 1
  ${AndIf} $R4 != ""
    FindFirst $R0 $R1 "$SMPROGRAMS\Reader*.lnk"
    readerl_startmenu_loop:
      StrCmp $R1 "" readerl_startmenu_done
      !insertmacro IsShortcutTarget "$SMPROGRAMS\$R1" "$R4\reader-desktop.exe"
      Pop $R8
      ${If} $R8 = 1
        Delete "$SMPROGRAMS\$R1"
      ${EndIf}
      FindNext $R0 $R1
      Goto readerl_startmenu_loop
    readerl_startmenu_done:
    FindClose $R0

    FindFirst $R0 $R1 "$DESKTOP\Reader*.lnk"
    readerl_desktop_loop:
      StrCmp $R1 "" readerl_desktop_done
      !insertmacro IsShortcutTarget "$DESKTOP\$R1" "$R4\reader-desktop.exe"
      Pop $R8
      ${If} $R8 = 1
        Delete "$DESKTOP\$R1"
      ${EndIf}
      FindNext $R0 $R1
      Goto readerl_desktop_loop
    readerl_desktop_done:
    FindClose $R0
  ${EndIf}

  readerl_migrate_done:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro ReaderLMigrateFromOldName
  ; Keep a product-name-independent record. Future releases can find the
  ; install directory even if PRODUCTNAME changes again.
  WriteRegStr SHCTX "${READER_STABLE_MANUPRODUCTKEY}" "" "$INSTDIR"

  ; 注册 legado:// 协议，让书源站点的「一键导入」能唤起本应用。
  ;
  ; 为什么不用 tauri-plugin-deep-link：那个插件主要服务 macOS/Linux 的 bundle
  ; 配置，Windows 上照样要写这些注册表项；本仓库已有自定义 NSIS 模板与 hook，
  ; 直接在这里注册少一层依赖，也能精确控制卸载时的清理条件。
  ;
  ; 只有**安装版**会注册 —— 便携包解压即用、不经过安装器，自然没有协议归属；
  ; 同一个协议也只能指向一个 exe（后安装的会覆盖先安装的）。
  WriteRegStr SHCTX "Software\Classes\legado" "URL Protocol" ""
  WriteRegStr SHCTX "Software\Classes\legado" "" "URL:Legado import protocol"
  WriteRegStr SHCTX "Software\Classes\legado\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
  WriteRegStr SHCTX "Software\Classes\legado\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; 只删仍然指向本安装目录的那份注册，避免把用户后来装的其它客户端（或者另一个
  ; 副本）的协议注册一并抹掉。
  ReadRegStr $R7 SHCTX "Software\Classes\legado\shell\open\command" ""
  ${If} $R7 == "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
    DeleteRegKey SHCTX "Software\Classes\legado"
  ${EndIf}
!macroend
