@echo off
setlocal enabledelayedexpansion

REM Attempt to change to system drive to avoid issues with current directory/drive
cd /d "%SystemDrive%" >nul 2>&1
if %errorlevel% neq 0 (
    echo Failed to change to %SystemDrive%.  Error code: %errorlevel%
)

REM Check if running as administrator (script should run as normal user)
net session >nul 2>&1
if %errorlevel% equ 0 (
    echo ERROR: This script is intended to be run as a user. Please run without administrator privileges.
    goto :error_exit
)

set FOUNDRY_DIR=%LOCALAPPDATA%\Programs\Foundry
set FOUNDRY_BIN=%FOUNDRY_DIR%\bin

echo ===================
echo Foundry Uninstaller
echo ===================
echo.

REM Check if Foundry is installed
if not exist "%FOUNDRY_BIN%" (
    echo Foundry installation not found at: %FOUNDRY_BIN%
    echo.
    echo Nothing to uninstall.
    goto :end
)

REM Display current version if available
if exist "%FOUNDRY_BIN%\forge.exe" (
    echo Current installed version:
    "%FOUNDRY_BIN%\forge.exe" --version 2>nul
    echo.
)

REM Confirm uninstallation
echo This will remove Foundry from your system.
echo Installation directory: %FOUNDRY_BIN%
echo.
set /p CONFIRM="Are you sure you want to uninstall Foundry? (Y/N): "
if /i not "!CONFIRM!"=="Y" (
    echo.
    echo Uninstallation cancelled.
    goto :end
)

REM Check for and terminate running Foundry processes
echo.
echo Checking for running Foundry processes...
echo.

set PROCESSES_KILLED=0
for %%e in (anvil cast chisel forge) do (
    tasklist /FI "IMAGENAME eq %%e.exe" 2>nul | find /I "%%e.exe" >nul 2>&1
    if !errorlevel! equ 0 (
        echo   Terminating %%e.exe...
        taskkill /F /IM "%%e.exe" >nul 2>&1
        if !errorlevel! equ 0 (
            echo   Terminated %%e.exe
            set PROCESSES_KILLED=1
        ) else (
            echo   WARNING: Failed to terminate %%e.exe
        )
    )
)

if !PROCESSES_KILLED! equ 0 (
    echo No running Foundry processes found.
    echo.
) else (
    echo.
    echo Waiting for processes to fully terminate...
    timeout /t 2 /nobreak >nul 2>&1
    echo.
)

REM Remove Foundry executables
echo Removing Foundry executables...
echo.

set REMOVAL_FAILED=0
for %%e in (anvil cast chisel forge) do (
    if exist "%FOUNDRY_BIN%\%%e.exe" (
        del /F /Q "%FOUNDRY_BIN%\%%e.exe" >nul 2>&1
        if !errorlevel! equ 0 (
            echo   Removed %%e.exe
        ) else (
            echo   ERROR: Failed to remove %%e.exe. Error code: !errorlevel!
            set REMOVAL_FAILED=1
        )
    )
)

if !REMOVAL_FAILED! equ 1 (
    echo.
    echo WARNING: Some executables could not be removed.
    echo This may be because they are still in use or protected.
    echo.
)

REM Remove Foundry from PATH
echo.
echo Removing Foundry from User PATH...
echo.
powershell -NoProfile -ExecutionPolicy Bypass -Command "try { $path = [Environment]::GetEnvironmentVariable('Path', 'User'); if ($null -eq $path) { Write-Host 'User PATH is empty'; exit 0 }; if ($path -like '*%FOUNDRY_BIN%*') { $pathArray = $path -split ';' | Where-Object { $_ -ne '' -and $_ -ne '%FOUNDRY_BIN%' }; $newPath = $pathArray -join ';'; [Environment]::SetEnvironmentVariable('Path', $newPath, 'User'); Write-Host 'Foundry removed from User PATH' } else { Write-Host 'Foundry not found in User PATH' }; exit 0 } catch { Write-Host \"ERROR: $_\"; exit 1 }" 2>&1
if %errorlevel% neq 0 (
    echo WARNING: Failed to remove Foundry from User PATH.
    echo You may need to manually remove it from your environment variables.
    echo.
)

REM Remove Foundry installation directory
echo.
echo Removing Foundry installation directory...
echo.
if exist "%FOUNDRY_DIR%" (
    rmdir /S /Q "%FOUNDRY_DIR%" >nul 2>&1
    if %errorlevel% equ 0 (
        echo Installation directory removed: %FOUNDRY_DIR%
        echo.
    ) else (
        echo WARNING: Failed to remove installation directory. Error code: %errorlevel%
        echo.
        echo This may be because files are in use or protected.
        echo You can manually delete: %FOUNDRY_DIR%
        echo.
        set REMOVAL_FAILED=1
    )
)

REM Display final status
echo.
if !REMOVAL_FAILED! equ 1 (
    echo ======================================
    echo Uninstallation completed with warnings
    echo ======================================
    echo.
    echo Some files or directories could not be removed.
    echo Please review the warnings above and take manual action if needed.
) else (
    echo ==========================================
    echo SUCCESS: Foundry uninstalled successfully!
    echo ==========================================
)
echo.
echo Note: You may need to restart your terminal or IDE for PATH changes to take effect.
echo.
goto :end

:error_exit
echo.
echo ======================================================
echo Uninstallation failed. Please review the errors above.
echo ======================================================
echo.
timeout /t 15 /nobreak
endlocal
exit /b 1

:end
timeout /t 15 /nobreak
endlocal
exit /b 0
