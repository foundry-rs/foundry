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

echo =========================
echo Foundry Installer/Updater
echo =========================
echo.

REM Detect latest Foundry version
set FOUNDRY_VERSION=
for /f "delims=" %%i in ('curl -Ls -o nul -w "%%{url_effective}" https://github.com/foundry-rs/foundry/releases/latest 2^>nul') do set FOUNDRY_LATEST_URL=%%i
if %errorlevel% neq 0 (
    echo ERROR: Failed to detect latest Foundry version.
    echo.
    echo Please check your internet connection and try again.
    echo If the problem persists, ensure curl is installed and accessible.
    goto :error_exit
)

for %%a in ("!FOUNDRY_LATEST_URL!") do set FOUNDRY_VERSION=%%~nxa
if "!FOUNDRY_VERSION!"=="" (
    echo ERROR: Failed to parse Foundry version from URL: !FOUNDRY_LATEST_URL!
    echo.
    echo Cannot proceed with installation.
    goto :error_exit
)

echo Latest Foundry version: !FOUNDRY_VERSION!
echo.

REM Set up paths and URLs
set FOUNDRY_URL=https://github.com/foundry-rs/foundry/releases/download/!FOUNDRY_VERSION!/foundry_!FOUNDRY_VERSION!_win32_amd64.zip
set FOUNDRY_DIR=%LOCALAPPDATA%\Programs\Foundry
set FOUNDRY_BIN=%FOUNDRY_DIR%\bin
set FOUNDRY_TEMP=%TEMP%\foundry_install_%RANDOM%_%RANDOM%
set FOUNDRY_BACKUP=%TEMP%\foundry_backup_%RANDOM%_%RANDOM%

REM Create installation directory if it doesn't exist
if not exist "%FOUNDRY_BIN%" (
    mkdir "%FOUNDRY_BIN%" >nul 2>&1
    if %errorlevel% neq 0 (
        echo ERROR: Failed to create installation directory: %FOUNDRY_BIN%
        echo Error code: %errorlevel%
        goto :error_exit
    )
)

REM Check current installation
set CURRENT_VERSION=
set NEEDS_BACKUP=0
if exist "%FOUNDRY_BIN%\forge.exe" (
    set "VERSION_TEMP=%TEMP%\foundry_version_%RANDOM%_%RANDOM%.txt"
    "%FOUNDRY_BIN%\forge.exe" --version > "!VERSION_TEMP!" 2>&1
    if %errorlevel% equ 0 (
        for /f "tokens=3 delims= " %%v in ('findstr /C:"forge Version:" "!VERSION_TEMP!"') do set CURRENT_VERSION=%%v
        del /F /Q "!VERSION_TEMP!" >nul 2>&1
        if not "!CURRENT_VERSION!"=="" (
            REM Extract version tag (e.g., v1.3.6 from 1.3.6-v1.3.6)
            for /f "tokens=2 delims=-" %%t in ("!CURRENT_VERSION!") do set CURRENT_VERSION_TAG=%%t
            if "!CURRENT_VERSION_TAG!"=="" set CURRENT_VERSION_TAG=!CURRENT_VERSION!
            echo Current installed version: !CURRENT_VERSION_TAG!
            echo.
            if "!CURRENT_VERSION_TAG!"=="!FOUNDRY_VERSION!" (
                echo Foundry !FOUNDRY_VERSION! is already installed and up to date.
                goto :end
            )
            echo Upgrading from !CURRENT_VERSION_TAG! to !FOUNDRY_VERSION!...
            echo.
            set NEEDS_BACKUP=1
        )
    ) else (
        if exist "!VERSION_TEMP!" del /F /Q "!VERSION_TEMP!" >nul 2>&1
    )
) else (
    echo No existing installation found.
    echo.
    echo Installing Foundry !FOUNDRY_VERSION!...
    echo.
)

REM Backup existing installation if upgrading
if !NEEDS_BACKUP! equ 1 (
    echo Creating backup of existing installation...
    echo.
    mkdir "%FOUNDRY_BACKUP%" >nul 2>&1
    if exist "%FOUNDRY_BIN%\anvil.exe" copy /Y "%FOUNDRY_BIN%\anvil.exe" "%FOUNDRY_BACKUP%\" >nul 2>&1
    if exist "%FOUNDRY_BIN%\cast.exe" copy /Y "%FOUNDRY_BIN%\cast.exe" "%FOUNDRY_BACKUP%\" >nul 2>&1
    if exist "%FOUNDRY_BIN%\chisel.exe" copy /Y "%FOUNDRY_BIN%\chisel.exe" "%FOUNDRY_BACKUP%\" >nul 2>&1
    if exist "%FOUNDRY_BIN%\forge.exe" copy /Y "%FOUNDRY_BIN%\forge.exe" "%FOUNDRY_BACKUP%\" >nul 2>&1

    REM Remove old executables
    if exist "%FOUNDRY_BIN%\anvil.exe" del /F /Q "%FOUNDRY_BIN%\anvil.exe" >nul 2>&1
    if exist "%FOUNDRY_BIN%\cast.exe" del /F /Q "%FOUNDRY_BIN%\cast.exe" >nul 2>&1
    if exist "%FOUNDRY_BIN%\chisel.exe" del /F /Q "%FOUNDRY_BIN%\chisel.exe" >nul 2>&1
    if exist "%FOUNDRY_BIN%\forge.exe" del /F /Q "%FOUNDRY_BIN%\forge.exe" >nul 2>&1
)

REM Download Foundry
echo Downloading Foundry !FOUNDRY_VERSION! from:
echo !FOUNDRY_URL!
echo.
curl -L -f --progress-bar -o "%FOUNDRY_TEMP%.zip" "!FOUNDRY_URL!" 2>&1
if %errorlevel% neq 0 (
    echo.
    echo ERROR: Failed to download Foundry. Error code: %errorlevel%
    echo.
    echo This could be due to:
    echo   - Network connectivity issues
    echo   - Invalid download URL
    goto :error_restore
)

REM Validate downloaded file exists and has content
if not exist "%FOUNDRY_TEMP%.zip" (
    echo ERROR: Downloaded file not found at %FOUNDRY_TEMP%.zip
    goto :error_restore
)
for %%A in ("%FOUNDRY_TEMP%.zip") do set FILESIZE=%%~zA
if !FILESIZE! lss 1000000 (
    echo ERROR: Downloaded file is too small ^(!FILESIZE! bytes^). Download may be corrupted.
    goto :error_restore
)

REM Extract Foundry
echo.
echo Extracting Foundry...
echo.
powershell -NoProfile -ExecutionPolicy Bypass -Command "try { Expand-Archive -Path '%FOUNDRY_TEMP%.zip' -DestinationPath '%FOUNDRY_TEMP%' -Force -ErrorAction Stop; exit 0 } catch { Write-Host \"ERROR: $_\"; exit 1 }" 2>&1
if %errorlevel% neq 0 (
    echo ERROR: Failed to extract Foundry archive. Error code: %errorlevel%
    goto :error_restore
)

REM Verify extracted files exist
if not exist "%FOUNDRY_TEMP%\forge.exe" (
    echo ERROR: forge.exe not found in extracted archive.
    echo.
    echo The archive structure may have changed or be corrupted.
    goto :error_restore
)

REM Install Foundry executables
echo Installing Foundry executables...
echo.

set INSTALL_FAILED=0
for %%e in (anvil cast chisel forge) do (
    if exist "%FOUNDRY_TEMP%\%%e.exe" (
        copy /Y "%FOUNDRY_TEMP%\%%e.exe" "%FOUNDRY_BIN%\" >nul 2>&1
        if !errorlevel! neq 0 (
            echo ERROR: Failed to install %%e.exe. Error code: !errorlevel!
            set INSTALL_FAILED=1
        ) else (
            echo   Installed %%e.exe
        )
    ) else (
        echo WARNING: %%e.exe not found in archive.
    )
)

if !INSTALL_FAILED! equ 1 (
    echo.
    echo Installation failed. Check if files are in use or if you have write permissions.
    goto :error_restore
)

REM Verify installation
echo.
echo Verifying installation...
echo.
if not exist "%FOUNDRY_BIN%\forge.exe" (
    echo ERROR: forge.exe not found after installation at %FOUNDRY_BIN%\forge.exe
    goto :error_restore
)

"%FOUNDRY_BIN%\forge.exe" --version 2>&1
if %errorlevel% neq 0 (
    echo ERROR: forge.exe failed to execute. Error code: %errorlevel%
    goto :error_restore
)

REM Update PATH environment variable
echo.
echo Updating PATH environment variable...
echo.
powershell -NoProfile -ExecutionPolicy Bypass -Command "try { $path = [Environment]::GetEnvironmentVariable('Path', 'User'); if ($null -eq $path) { $path = '' }; if ($path -notlike '*%FOUNDRY_BIN%*') { $newPath = if ($path -eq '') { '%FOUNDRY_BIN%' } else { $path.TrimEnd(';') + ';%FOUNDRY_BIN%' }; [Environment]::SetEnvironmentVariable('Path', $newPath, 'User'); Write-Host 'Foundry added to User PATH permanently' } else { Write-Host 'Foundry already in User PATH' }; exit 0 } catch { Write-Host \"ERROR: $_\"; exit 1 }" 2>&1
if %errorlevel% neq 0 (
    echo WARNING: Failed to update User PATH environment variable.
    echo You may need to manually add %FOUNDRY_BIN% to your PATH.
    echo.
)

REM Update PATH for current session
set "PATH=%PATH%;%FOUNDRY_BIN%"

REM Success! Clean up temporary files and backup
call :cleanup
if exist "%FOUNDRY_BACKUP%" rmdir /S /Q "%FOUNDRY_BACKUP%" >nul 2>&1

echo.
echo ========================================
echo SUCCESS: Foundry !FOUNDRY_VERSION! installed successfully!
echo ========================================
echo.
echo Installation directory: %FOUNDRY_BIN%
echo.
echo Note: You may need to restart your terminal or IDE to use Foundry commands.
echo       In the current session, Foundry commands are already available.
echo.
goto :end

:error_restore
REM Attempt to restore backup if upgrade failed
if !NEEDS_BACKUP! equ 1 (
    if exist "%FOUNDRY_BACKUP%" (
        echo.
        echo Attempting to restore previous installation...
        if exist "%FOUNDRY_BACKUP%\anvil.exe" copy /Y "%FOUNDRY_BACKUP%\anvil.exe" "%FOUNDRY_BIN%\" >nul 2>&1
        if exist "%FOUNDRY_BACKUP%\cast.exe" copy /Y "%FOUNDRY_BACKUP%\cast.exe" "%FOUNDRY_BIN%\" >nul 2>&1
        if exist "%FOUNDRY_BACKUP%\chisel.exe" copy /Y "%FOUNDRY_BACKUP%\chisel.exe" "%FOUNDRY_BIN%\" >nul 2>&1
        if exist "%FOUNDRY_BACKUP%\forge.exe" copy /Y "%FOUNDRY_BACKUP%\forge.exe" "%FOUNDRY_BIN%\" >nul 2>&1
        echo Previous installation restored.
        echo.
    )
)

:error_cleanup
REM Clean up temporary files and backup
call :cleanup
if exist "%FOUNDRY_BACKUP%" rmdir /S /Q "%FOUNDRY_BACKUP%" >nul 2>&1

:error_exit
echo.
echo ====================================================
echo Installation failed. Please review the errors above.
echo ====================================================
echo.
echo For help, visit: https://github.com/foundry-rs/foundry/issues
echo.
timeout /t 15 /nobreak
endlocal
exit /b 1

:end
timeout /t 15 /nobreak
endlocal
exit /b 0

:cleanup
REM Subroutine to clean up temporary files
if exist "%FOUNDRY_TEMP%.zip" del /F /Q "%FOUNDRY_TEMP%.zip" >nul 2>&1
if exist "%FOUNDRY_TEMP%" rmdir /S /Q "%FOUNDRY_TEMP%" >nul 2>&1
exit /b 0
