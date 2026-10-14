@echo off
setlocal
echo Revate — Windows installer helper
echo ================================
echo.

REM Locate the installer in the same folder as this script
set "SCRIPT_DIR=%~dp0"
set "INSTALLER="

for %%F in ("%SCRIPT_DIR%Revate-Setup-*.exe") do (
    set "INSTALLER=%%F"
    goto :found
)

echo Could not find Revate-Setup-*.exe in the same folder as this script.
echo Please make sure both files are in the same folder.
pause
exit /b 1

:found
echo Found installer: %INSTALLER%
echo.

echo Removing the "downloaded from internet" flag...
powershell -NoProfile -Command "Unblock-File -Path '%INSTALLER%'"

echo Launching installer...
"%INSTALLER%"

echo.
echo If installation succeeded, Revate will be in your Start menu.
pause