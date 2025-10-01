#!/usr/bin/env pwsh
#Requires -Version 5.1

# PSScriptAnalyzer suppressions - Write-Host is intentional for installer output formatting
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingWriteHost', '', Justification='Installer requires formatted colored output for user')]

<#
.SYNOPSIS
    Bootstrap installer for foundryup on Windows.

.DESCRIPTION
    This script sets up foundryup on your Windows system by:
    - Creating the .foundry directory structure
    - Downloading or installing foundryup.ps1
    - Adding the foundry bin directory to your PATH

.EXAMPLE
    .\install.ps1
    Installs foundryup to the default location

.EXAMPLE
    irm https://foundry.paradigm.xyz/install.ps1 | iex
    One-line installation command (when hosted)
#>

[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

#region Configuration

$Script:BASE_DIR = if ($env:XDG_CONFIG_HOME) { $env:XDG_CONFIG_HOME } else { $env:USERPROFILE }
$Script:FOUNDRY_DIR = if ($env:FOUNDRY_DIR) { $env:FOUNDRY_DIR } else { Join-Path $BASE_DIR ".foundry" }
$Script:FOUNDRY_BIN_DIR = Join-Path $FOUNDRY_DIR "bin"
$Script:FOUNDRY_MAN_DIR = Join-Path $FOUNDRY_DIR "share\man\man1"

# URL for the foundryup.ps1 script
$Script:FOUNDRYUP_URL = "https://raw.githubusercontent.com/foundry-rs/foundry/master/foundryup/foundryup.ps1"
$Script:FOUNDRYUP_PATH = Join-Path $FOUNDRY_BIN_DIR "foundryup.ps1"

#endregion

#region Utility Functions

function Write-Info {
    param([string]$Message)
    Write-Host "foundryup installer: $Message" -ForegroundColor Cyan
}

function Write-Success {
    param([string]$Message)
    Write-Host "foundryup installer: $Message" -ForegroundColor Green
}

function Write-Warn {
    param([string]$Message)
    Write-Warning "foundryup installer: $Message"
}

function Write-Err {
    param([string]$Message)
    Write-Error "foundryup installer: $Message" -ErrorAction Stop
}

function Test-CommandExists {
    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSUseSingularNouns', '', Justification='Test-CommandExists is more readable than Test-CommandExist')]
    param([string]$Command)
    
    if ([string]::IsNullOrWhiteSpace($Command)) {
        return $false
    }
    
    $null -ne (Get-Command $Command -ErrorAction SilentlyContinue)
}

function Add-ToUserPath {
    param([string]$Directory)

    # Get current user PATH
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    
    # Check if directory is already in PATH
    if ($userPath -split ';' | Where-Object { $_ -eq $Directory }) {
        Write-Info "foundryup bin directory is already in PATH"
        return $false
    }

    # Add to PATH
    $newPath = if ($userPath) { "$userPath;$Directory" } else { $Directory }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    
    # Update current session PATH
    $env:Path = "$env:Path;$Directory"
    
    Write-Success "added $Directory to user PATH"
    return $true
}

function Test-IsInPath {
    param([string]$Directory)
    
    $paths = $env:Path -split ';'
    return ($paths -contains $Directory)
}

#endregion

#region Main Installation

function Install-Foundryup {
    Write-Info "Installing foundryup..."
    Write-Host ""

    # Create directories
    Write-Info "creating directory structure..."
    
    if (-not (Test-Path $Script:FOUNDRY_DIR)) {
        New-Item -ItemType Directory -Path $Script:FOUNDRY_DIR -Force | Out-Null
        Write-Info "created $Script:FOUNDRY_DIR"
    }
    
    if (-not (Test-Path $Script:FOUNDRY_BIN_DIR)) {
        New-Item -ItemType Directory -Path $Script:FOUNDRY_BIN_DIR -Force | Out-Null
        Write-Info "created $Script:FOUNDRY_BIN_DIR"
    }
    
    if (-not (Test-Path $Script:FOUNDRY_MAN_DIR)) {
        New-Item -ItemType Directory -Path $Script:FOUNDRY_MAN_DIR -Force | Out-Null
        Write-Info "created $Script:FOUNDRY_MAN_DIR"
    }

    Write-Host ""

    # Check if we're running from the foundryup repository
    $localFoundryupPath = Join-Path $PSScriptRoot "foundryup.ps1"
    
    if (Test-Path $localFoundryupPath) {
        # Use local version
        Write-Info "found local foundryup.ps1, copying to $Script:FOUNDRYUP_PATH"
        Copy-Item -Path $localFoundryupPath -Destination $Script:FOUNDRYUP_PATH -Force
        Write-Success "installed local foundryup.ps1"
    }
    else {
        # Download from GitHub
        Write-Info "downloading foundryup.ps1 from GitHub..."
        
        try {
            $ProgressPreference = 'SilentlyContinue'
            Invoke-WebRequest -Uri $Script:FOUNDRYUP_URL -OutFile $Script:FOUNDRYUP_PATH -UseBasicParsing
            $ProgressPreference = 'Continue'
            Write-Success "downloaded foundryup.ps1"
        }
        catch {
            Write-Err "failed to download foundryup.ps1: $_"
        }
    }

    # Install foundryup.cmd wrapper for easier usage
    $localCmdPath = Join-Path $PSScriptRoot "foundryup.cmd"
    $installedCmdPath = Join-Path $Script:FOUNDRY_BIN_DIR "foundryup.cmd"
    
    if (Test-Path $localCmdPath) {
        Write-Info "installing foundryup.cmd wrapper..."
        Copy-Item -Path $localCmdPath -Destination $installedCmdPath -Force
        Write-Success "installed foundryup.cmd wrapper"
    }
    else {
        # Create the wrapper if it doesn't exist
        Write-Info "creating foundryup.cmd wrapper..."
        $wrapperContent = @'
@echo off
REM Wrapper script for foundryup.ps1 on Windows
REM This allows users to simply type "foundryup" instead of "foundryup.ps1"

powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0foundryup.ps1" %*
'@
        Set-Content -Path $installedCmdPath -Value $wrapperContent -Encoding ASCII
        Write-Success "created foundryup.cmd wrapper"
    }

    Write-Host ""

    # Add to PATH
    Write-Info "configuring PATH..."
    $addedToPath = Add-ToUserPath -Directory $Script:FOUNDRY_BIN_DIR

    Write-Host ""

    # Check for Git
    if (-not (Test-CommandExists "git")) {
        Write-Warn "Git is not installed. Foundryup requires Git for building from source."
        Write-Warn "Download Git from: https://git-scm.com/download/win"
    }

    # Check for Rust/Cargo (optional, only needed for building from source)
    if (-not (Test-CommandExists "cargo")) {
        Write-Info "Rust/Cargo is not installed (optional, only needed for building from source)."
        Write-Info "To build from source, install Rust from: https://rustup.rs/"
    }

    Write-Host ""
    Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Green
    Write-Success "foundryup installation complete!"
    Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Green
    Write-Host ""

    if ($addedToPath) {
        Write-Info "foundryup has been added to your PATH."
        Write-Info "You may need to restart your terminal for PATH changes to take effect."
        Write-Host ""
    }

    Write-Info "To install Foundry, run:"
    Write-Host "    foundryup" -ForegroundColor Yellow
    Write-Host ""
    Write-Info "Or directly:"
    Write-Host "    foundryup.ps1" -ForegroundColor Yellow
    Write-Host ""
    Write-Info "For help, run:"
    Write-Host "    foundryup -Help" -ForegroundColor Yellow
    Write-Host ""
}

#endregion

#region Entry Point

try {
    Install-Foundryup
}
catch {
    Write-Err "Installation failed: $_"
    exit 1
}

#endregion

