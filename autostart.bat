@echo off
:: ============================================================
:: Servynex - demarrage automatique a l'ouverture de session
:: ON  : cree la tache planifiee "Servynex" (sans droits administrateur)
:: OFF : la supprime. Servynex deja lance continue jusqu'a la
::       fermeture de session.
:: ASCII uniquement : un accent desynchronise l'analyse de cmd.exe.
:: ============================================================
setlocal
chcp 65001 >nul
cd /d "%~dp0"
title Servynex - demarrage automatique

set "ETAT=OFF"
for /f %%E in ('powershell -NoProfile -Command "$t = Get-ScheduledTask -TaskName Servynex -ErrorAction SilentlyContinue; if ($t -and $t.State -ne 'Disabled') { 'ON' } else { 'OFF' }"') do set "ETAT=%%E"

echo ==========================================
echo  Servynex - demarrage automatique
echo  Etat actuel : %ETAT%
echo ==========================================
echo.

:question
set "CHOIX="
set /p "CHOIX=Tapez ON ou OFF : "
if /i "%CHOIX%"=="ON" goto :on
if /i "%CHOIX%"=="OFF" goto :off
echo Reponse non reconnue.
goto :question

:on
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\install-autostart.ps1"
if errorlevel 1 (
  echo [ERREUR] La tache n'a pas pu etre creee.
  goto :fin
)
:: Lance Servynex tout de suite s'il ne tourne pas deja.
netstat -ano | findstr /i "LISTENING" | findstr "127.0.0.1:9000 " >nul
if errorlevel 1 (
  powershell -NoProfile -Command "Start-ScheduledTask -TaskName Servynex"
  echo Servynex lance : http://localhost:9000
)
echo.
echo ON : Servynex demarrera a chaque ouverture de session.
goto :fin

:off
powershell -NoProfile -Command "if (Get-ScheduledTask -TaskName Servynex -ErrorAction SilentlyContinue) { Unregister-ScheduledTask -TaskName Servynex -Confirm:$false }"
if errorlevel 1 (
  echo [ERREUR] La tache n'a pas pu etre supprimee.
  goto :fin
)
echo OFF : Servynex ne demarrera plus tout seul.
echo Les projets "Lancer au demarrage du PC" ne demarreront donc plus non plus.
echo Servynex deja lance continue de tourner jusqu'a la fermeture de session.

:fin
echo.
pause
