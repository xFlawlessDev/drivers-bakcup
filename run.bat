@echo off
setlocal

rem ============================================================
rem  Driver Backup - convenience launcher
rem  Runs the release build with admin rights.
rem ============================================================

set "EXE=%~dp0target\release\driver-backup.exe"
set "OUTDIR=%~dp0driver_backup"

rem Build the release binary if it is missing.
if not exist "%EXE%" (
    echo Release binary not found, building...
    pushd "%~dp0"
    cargo build --release
    if errorlevel 1 (
        echo Build failed.
        popd
        pause
        exit /b 1
    )
    popd
)

rem Re-launch elevated if not already running as Administrator.
net session >nul 2>&1
if %errorlevel% neq 0 (
    echo Requesting administrator privileges...
    powershell -NoProfile -Command "Start-Process -Verb RunAs -FilePath '%EXE%' -ArgumentList 'backup','--output','%OUTDIR%','--verbose'"
    exit /b
)

"%EXE%" backup --output "%OUTDIR%" --verbose
pause
