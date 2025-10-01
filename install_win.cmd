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

echo Detecting latest Foundry version...
echo.
set FOUNDRY_VERSION=
for /f "delims=" %%i in ('curl -Ls -o nul -w "%%{url_effective}" https://github.com/foundry-rs/foundry/releases/latest') do set FOUNDRY_LATEST_URL=%%i
if %errorlevel% neq 0 (
    echo ERROR: Failed to detect latest Foundry version. Cannot proceed with installation.
    echo.
    echo Please check your internet connection and try again.
    goto :skip_foundry
)

for %%a in ("!FOUNDRY_LATEST_URL!") do set FOUNDRY_VERSION=%%~nxa
if "!FOUNDRY_VERSION!"=="" (
    echo ERROR: Failed to parse Foundry version from URL: !FOUNDRY_LATEST_URL!
    echo.
    echo Cannot proceed with installation.
    goto :skip_foundry
)

echo Latest Foundry version detected: !FOUNDRY_VERSION!
echo.

echo Installing/Upgrading Foundry...
echo.
set FOUNDRY_URL=https://github.com/foundry-rs/foundry/releases/download/!FOUNDRY_VERSION!/foundry_!FOUNDRY_VERSION!_win32_amd64.zip
set FOUNDRY_DIR=%LOCALAPPDATA%\Programs\Foundry
set FOUNDRY_BIN=%FOUNDRY_DIR%\bin
set FOUNDRY_TEMP=%TEMP%\foundry_install_%RANDOM%_%RANDOM%

echo Creating Foundry directory...
if not exist "%FOUNDRY_BIN%" mkdir "%FOUNDRY_BIN%" >nul 2>&1

echo Checking for existing Foundry installation...
echo.
set CURRENT_VERSION=
if exist "%FOUNDRY_BIN%\forge.exe" (
    echo Found existing Foundry installation. Checking version...
    echo.
    for /f "tokens=2 delims= " %%v in ('"%FOUNDRY_BIN%\forge.exe" --version 2^>nul ^| findstr /C:"forge"') do set CURRENT_VERSION=%%v
    if not "!CURRENT_VERSION!"=="" (
        echo Current version: !CURRENT_VERSION!
        echo.
        if "!CURRENT_VERSION!"=="!FOUNDRY_VERSION!" (
            echo Foundry !FOUNDRY_VERSION! is already installed and up to date.
            echo.
            echo No installation needed.
            goto :skip_foundry
        )
        echo Newer version available: !FOUNDRY_VERSION!
        echo.
    )
    echo Removing old version...
    echo.
    if exist "%FOUNDRY_BIN%\anvil.exe" del /F /Q "%FOUNDRY_BIN%\anvil.exe" >nul 2>&1
    if exist "%FOUNDRY_BIN%\cast.exe" del /F /Q "%FOUNDRY_BIN%\cast.exe" >nul 2>&1
    if exist "%FOUNDRY_BIN%\chisel.exe" del /F /Q "%FOUNDRY_BIN%\chisel.exe" >nul 2>&1
    if exist "%FOUNDRY_BIN%\forge.exe" del /F /Q "%FOUNDRY_BIN%\forge.exe" >nul 2>&1
    echo Old version removed.
    echo.
) else (
    echo No existing Foundry installation found.
    echo.
)

echo Downloading Foundry !FOUNDRY_VERSION!...
echo.
curl -L -o "%FOUNDRY_TEMP%.zip" "!FOUNDRY_URL!" >nul 2>&1
if %errorlevel% neq 0 (
    echo Warning: Failed to download Foundry.  Error code: %errorlevel%
    goto :skip_foundry
)

echo Extracting Foundry...
echo.
powershell -Command "Expand-Archive -Path '%FOUNDRY_TEMP%.zip' -DestinationPath '%FOUNDRY_TEMP%' -Force" >nul 2>&1
if %errorlevel% neq 0 (
    echo Warning: Failed to extract Foundry archive.
    goto :cleanup_foundry
)

echo Installing Foundry executables...
echo.
copy /Y "%FOUNDRY_TEMP%\anvil.exe" "%FOUNDRY_BIN%\" >nul 2>&1
if %errorlevel% neq 0 (
    echo Warning: Failed to install anvil.exe. Error code: %errorlevel%
    goto :cleanup_foundry
)
copy /Y "%FOUNDRY_TEMP%\cast.exe" "%FOUNDRY_BIN%\" >nul 2>&1
if %errorlevel% neq 0 (
    echo Warning: Failed to install cast.exe. Error code: %errorlevel%
    goto :cleanup_foundry
)
copy /Y "%FOUNDRY_TEMP%\chisel.exe" "%FOUNDRY_BIN%\" >nul 2>&1
if %errorlevel% neq 0 (
    echo Warning: Failed to install chisel.exe. Error code: %errorlevel%
    goto :cleanup_foundry
)
copy /Y "%FOUNDRY_TEMP%\forge.exe" "%FOUNDRY_BIN%\" >nul 2>&1
if %errorlevel% neq 0 (
    echo Warning: Failed to install forge.exe. Error code: %errorlevel%
    goto :cleanup_foundry
)

echo Verifying installation...
echo.
if exist "%FOUNDRY_BIN%\forge.exe" (
    "%FOUNDRY_BIN%\forge.exe" --version
) else (
    echo Warning: forge.exe not found after installation. Error code: %errorlevel%
    goto :cleanup_foundry
)

echo Adding Foundry to User PATH permanently...
echo.
powershell -Command "$path = [Environment]::GetEnvironmentVariable('Path', 'User'); if ($path -notlike '*%FOUNDRY_BIN%*') { [Environment]::SetEnvironmentVariable('Path', $path + ';%FOUNDRY_BIN%', 'User'); Write-Host 'Foundry added to User PATH permanently' } else { Write-Host 'Foundry already in User PATH' }" >nul 2>&1

echo Setting PATH for current session...
echo.
set "PATH=%PATH%;%FOUNDRY_BIN%"

echo Foundry !FOUNDRY_VERSION! installed successfully!
echo.
echo Installation directory: %FOUNDRY_BIN%
echo.
echo Note: You may need to restart your terminal or IDE to use Foundry commands.
echo.

:cleanup_foundry
if exist "%FOUNDRY_TEMP%.zip" del /F /Q "%FOUNDRY_TEMP%.zip" >nul 2>&1
if exist "%FOUNDRY_TEMP%" rmdir /S /Q "%FOUNDRY_TEMP%" >nul 2>&1

echo Installation complete!

:skip_foundry

timeout /t 10 /nobreak
endlocal
exit /b 0
