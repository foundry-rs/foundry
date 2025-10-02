@echo off
setlocal enabledelayedexpansion

cd /d "%SystemDrive%" >nul 2>&1
if %errorlevel% neq 0 (
    echo Failed to change to %SystemDrive%.  Error code: %errorlevel%
)

net session >nul 2>&1
if %errorlevel% equ 0 (
    echo This script is intended to be run as a user. Please run without administrator privileges.
    timeout /t 10 /nobreak
    endlocal
    exit /b 1
)

set FOUNDRY_DIR=%LOCALAPPDATA%\Programs\Foundry
set FOUNDRY_BIN=%FOUNDRY_DIR%\bin

echo Foundry Uninstaller
echo ===================
echo.

if not exist "%FOUNDRY_BIN%" (
    echo Foundry installation not found at: %FOUNDRY_BIN%
    echo.
    echo Nothing to uninstall.
    goto :end
)

if exist "%FOUNDRY_BIN%\forge.exe" (
    echo Current installed version:
    "%FOUNDRY_BIN%\forge.exe" --version 2>nul
    echo.
)

echo This will remove Foundry from your system.
echo Installation directory: %FOUNDRY_BIN%
echo.
set /p CONFIRM="Are you sure you want to uninstall Foundry? (Y/N): "
if /i not "!CONFIRM!"=="Y" (
    echo Uninstallation cancelled.
    goto :end
)

echo Removing Foundry executables...
echo.
if exist "%FOUNDRY_BIN%\anvil.exe" (
    del /F /Q "%FOUNDRY_BIN%\anvil.exe" >nul 2>&1
    if %errorlevel% equ 0 (
        echo   - anvil.exe removed
        echo.
    ) else (
        echo   - Warning: Failed to remove anvil.exe.  Error code: %errorlevel%
        echo.
    )
)
if exist "%FOUNDRY_BIN%\cast.exe" (
    del /F /Q "%FOUNDRY_BIN%\cast.exe" >nul 2>&1
    if %errorlevel% equ 0 (
        echo   - cast.exe removed
        echo.
    ) else (
        echo   - Warning: Failed to remove cast.exe.  Error code: %errorlevel%
        echo.
    )
)
if exist "%FOUNDRY_BIN%\chisel.exe" (
    del /F /Q "%FOUNDRY_BIN%\chisel.exe" >nul 2>&1
    if %errorlevel% equ 0 (
        echo   - chisel.exe removed
        echo.
    ) else (
        echo   - Warning: Failed to remove chisel.exe.  Error code: %errorlevel%
        echo.
    )
)
if exist "%FOUNDRY_BIN%\forge.exe" (
    del /F /Q "%FOUNDRY_BIN%\forge.exe" >nul 2>&1
    if %errorlevel% equ 0 (
        echo   - forge.exe removed
        echo.
    ) else (
        echo   - Warning: Failed to remove forge.exe.  Error code: %errorlevel%
        echo.
    )
)

echo Removing Foundry from User PATH...
echo.
powershell -Command "$path = [Environment]::GetEnvironmentVariable('Path', 'User'); if ($path -like '*%FOUNDRY_BIN%*') { $newPath = ($path -split ';' | Where-Object { $_ -ne '%FOUNDRY_BIN%' }) -join ';'; [Environment]::SetEnvironmentVariable('Path', $newPath, 'User'); Write-Host 'Foundry removed from User PATH' } else { Write-Host 'Foundry not found in User PATH' }" 2>nul
if %errorlevel% neq 0 (
    echo Warning: Failed to remove Foundry from User PATH.  Error code: %errorlevel%
    echo You may need to remove it manually from your environment variables.
    echo.
)

echo Removing Foundry installation directory...
echo.
if exist "%FOUNDRY_DIR%" (
    rmdir /S /Q "%FOUNDRY_DIR%" >nul 2>&1
    if %errorlevel% equ 0 (
        echo Installation directory removed: %FOUNDRY_DIR%
        echo.
    ) else (
        echo Warning: Failed to remove installation directory.  Error code: %errorlevel%
        echo You may need to manually delete: %FOUNDRY_DIR%
        echo.
    )
)

echo Foundry has been uninstalled successfully!
echo.
echo Note: You may need to restart your terminal or IDE for PATH changes to take effect.
echo.

:end
timeout /t 10 /nobreak
endlocal
exit /b 0
