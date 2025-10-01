#!/usr/bin/env pwsh
#Requires -Version 5.1

<#
.SYNOPSIS
    The installer for Foundry.

.DESCRIPTION
    Update or revert to a specific Foundry version with ease.
    By default, the latest stable version is installed from built binaries.

.PARAMETER Help
    Print help information

.PARAMETER Version
    Print the version of foundryup

.PARAMETER Update
    Update foundryup to the latest version

.PARAMETER Install
    Install a specific version from built binaries

.PARAMETER List
    List versions installed from built binaries

.PARAMETER Use
    Use a specific installed version from built binaries

.PARAMETER Branch
    Build and install a specific branch

.PARAMETER PR
    Build and install a specific Pull Request

.PARAMETER Commit
    Build and install a specific commit

.PARAMETER Repo
    Build and install from a remote GitHub repo (uses default branch if no other options are set)

.PARAMETER Path
    Build and install a local repository

.PARAMETER Jobs
    Number of CPUs to use for building Foundry (default: all CPUs)

.PARAMETER Force
    Skip SHA verification for downloaded binaries (INSECURE - use with caution)

.PARAMETER Arch
    Install a specific architecture (supports amd64 and arm64)

.PARAMETER Platform
    Install a specific platform (supports win32, linux, darwin and alpine)

.EXAMPLE
    .\foundryup.ps1
    Installs the latest stable version

.EXAMPLE
    .\foundryup.ps1 -Install 0.2.0
    Installs version 0.2.0

.EXAMPLE
    .\foundryup.ps1 -List
    Lists all installed versions
#>

[CmdletBinding()]
[System.Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSReviewUnusedParameter', '', Justification='Parameters are used in Main function')]
param(
    [Parameter()][switch]$Help,
    [Parameter()][switch]$Version,
    [Parameter()][switch]$Update,
    [Parameter()][string]$Install,
    [Parameter()][switch]$List,
    [Parameter()][string]$Use,
    [Parameter()][string]$Branch,
    [Parameter()][string]$PR,
    [Parameter()][string]$Commit,
    [Parameter()][string]$Repo,
    [Parameter()][string]$Path,
    [Parameter()][int]$Jobs,
    [Parameter()][switch]$Force,
    [Parameter()][string]$Arch,
    [Parameter()][string]$Platform
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# NOTE: if you make modifications to this script, please increment the version number.
# WARNING: the SemVer pattern: major.minor.patch must be followed as we use it to determine if the script is up to date.
$Script:FOUNDRYUP_INSTALLER_VERSION = "1.3.0"

$Script:BASE_DIR = if ($env:XDG_CONFIG_HOME) { $env:XDG_CONFIG_HOME } else { $env:USERPROFILE }
$Script:FOUNDRY_DIR = if ($env:FOUNDRY_DIR) { $env:FOUNDRY_DIR } else { Join-Path $BASE_DIR ".foundry" }
$Script:FOUNDRY_VERSIONS_DIR = Join-Path $FOUNDRY_DIR "versions"
$Script:FOUNDRY_BIN_DIR = Join-Path $FOUNDRY_DIR "bin"
$Script:FOUNDRY_MAN_DIR = Join-Path $FOUNDRY_DIR "share\man\man1"
$Script:FOUNDRY_BIN_URL = "https://raw.githubusercontent.com/foundry-rs/foundry/master/foundryup/foundryup"
$Script:FOUNDRY_BIN_PATH = Join-Path $FOUNDRY_BIN_DIR "foundryup.ps1"
$Script:FOUNDRYUP_JOBS = $Jobs
$Script:FOUNDRYUP_IGNORE_VERIFICATION = $Force.IsPresent

$Script:BINS = @("forge", "cast", "anvil", "chisel")
$Script:HASH_NAMES = @()
$Script:HASH_VALUES = @()

# Detect Windows for cross-version compatibility
# Use script-scoped variable to avoid conflicts with built-in $IsWindows in PS 6.0+
if (Test-Path variable:global:IsWindows) {
    $Script:IsWindowsOS = $IsWindows
} else {
    # PowerShell 5.1 compatibility
    $Script:IsWindowsOS = $env:OS -match "Windows"
}

if (-not $env:RUSTFLAGS) {
    $env:RUSTFLAGS = "-C target-cpu=native"
}

#region Utility Functions

function Write-Say {
    param([string]$Message)
    Write-Output "foundryup: $Message"
}

function Write-Warn {
    param([string]$Message)
    Write-Warning "foundryup: warning: $Message"
}

function Write-Err {
    param([string]$Message)
    Write-Error "foundryup: $Message" -ErrorAction Stop
}

function Test-CommandExists {
    param([string]$Command)
    $null -ne (Get-Command $Command -ErrorAction SilentlyContinue)
}

function Assert-CommandExists {
    param([string]$Command)
    if (-not (Test-CommandExists $Command)) {
        Write-Err "need '$Command' (command not found)"
    }
}

function Invoke-Ensure {
    param(
        [Parameter(Mandatory = $true)]
        [scriptblock]$ScriptBlock,
        [string]$ErrorMessage
    )

    try {
        & $ScriptBlock
        if ($LASTEXITCODE -ne 0 -and $null -ne $LASTEXITCODE) {
            throw "Command exited with code $LASTEXITCODE"
        }
    }
    catch {
        if ($ErrorMessage) {
            Write-Err $ErrorMessage
        }
        else {
            Write-Err "command failed: $($ScriptBlock.ToString())"
        }
    }
}

function Get-FileSha256 {
    param([string]$FilePath)
    $hash = Get-FileHash -Path $FilePath -Algorithm SHA256
    return $hash.Hash.ToLower()
}

function Invoke-Download {
    param(
        [string]$Url,
        [string]$OutFile
    )

    try {
        if ($OutFile) {
            Write-Say "downloading from $Url"
            $ProgressPreference = 'SilentlyContinue'
            Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing
            $ProgressPreference = 'Continue'
        }
        else {
            $ProgressPreference = 'SilentlyContinue'
            $response = Invoke-WebRequest -Uri $Url -UseBasicParsing
            $ProgressPreference = 'Continue'
            return $response.Content
        }
    }
    catch {
        Write-Err "download failed: $Url - $_"
    }
}

function Compare-Version {
    param(
        [string]$Version1,
        [string]$Version2
    )

    if ($Version1 -eq $Version2) {
        return $false
    }

    $v1Parts = $Version1.Split('.')
    $v2Parts = $Version2.Split('.')

    $major1 = [int]$v1Parts[0]
    $minor1 = [int]$v1Parts[1]
    $patch1 = [int]$v1Parts[2]

    $major2 = [int]$v2Parts[0]
    $minor2 = [int]$v2Parts[1]
    $patch2 = [int]$v2Parts[2]

    if ($major1 -gt $major2) { return $true }
    if ($major1 -lt $major2) { return $false }
    if ($minor1 -gt $minor2) { return $true }
    if ($minor1 -lt $minor2) { return $false }
    if ($patch1 -gt $patch2) { return $true }
    if ($patch1 -lt $patch2) { return $false }

    return $false
}

function Test-BinsInUse {
    foreach ($bin in $Script:BINS) {
        $processes = Get-Process -Name $bin -ErrorAction SilentlyContinue
        if ($processes) {
            Write-Err "Error: '$bin' is currently running. Please stop the process and try again."
        }
    }
}

#endregion

#region Main Functions

function Show-Banner {
    Write-Output @"


.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx

 +-+ +-+ - - +++ +-+ --+ - -         Portable and modular toolkit
 ¦¦  ¦ ¦ ¦ ¦ ¦¦¦  ¦¦ ¦-+ +-+    for Ethereum Application Development
 +   +-+ +-+ +++ --+ -+-  -                 written in Rust.

.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx

Repo       : https://github.com/foundry-rs/foundry
Book       : https://book.getfoundry.sh/
Chat       : https://t.me/foundry_rs/
Support    : https://t.me/foundry_support/
Contribute : https://github.com/foundry-rs/foundry/blob/master/CONTRIBUTING.md

.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx


"@
}

function Show-Usage {
    Write-Output @"
The installer for Foundry.

Update or revert to a specific Foundry version with ease.

By default, the latest stable version is installed from built binaries.

USAGE:
    foundryup.ps1 [OPTIONS]

OPTIONS:
    -Help           Print help information
    -Version        Print the version of foundryup
    -Update         Update foundryup to the latest version
    -Install        Install a specific version from built binaries
    -List           List versions installed from built binaries
    -Use            Use a specific installed version from built binaries
    -Branch         Build and install a specific branch
    -PR             Build and install a specific Pull Request
    -Commit         Build and install a specific commit
    -Repo           Build and install from a remote GitHub repo (uses default branch if no other options are set)
    -Path           Build and install a local repository
    -Jobs           Number of CPUs to use for building Foundry (default: all CPUs)
    -Force          Skip SHA verification for downloaded binaries (INSECURE - use with caution)
    -Arch           Install a specific architecture (supports amd64 and arm64)
    -Platform       Install a specific platform (supports win32, linux, darwin and alpine)
"@
}

function Show-Version {
    Write-Say $Script:FOUNDRYUP_INSTALLER_VERSION
    exit 0
}

function Update-Foundryup {
    Write-Say "updating foundryup..."

    $currentVersion = $Script:FOUNDRYUP_INSTALLER_VERSION

    # Download the new version - get the bash script and extract version
    $tmpFile = New-TemporaryFile
    Invoke-Download -Url $Script:FOUNDRY_BIN_URL -OutFile $tmpFile.FullName

    # Extract new version from downloaded file
    $content = Get-Content -Path $tmpFile.FullName -Raw
    if ($content -match 'FOUNDRYUP_INSTALLER_VERSION="([0-9]+\.[0-9]+\.[0-9]+)"') {
        $newVersion = $Matches[1]
    }
    else {
        Write-Warn "could not determine new foundryup version. Exiting."
        Remove-Item -Path $tmpFile.FullName -Force
        exit 0
    }

    # If the new version is not greater than the current version, skip the update
    if (-not (Compare-Version -Version1 $newVersion -Version2 $currentVersion)) {
        Write-Say "foundryup is already up to date (installed: $currentVersion, remote: $newVersion)."
        Remove-Item -Path $tmpFile.FullName -Force
        exit 0
    }

    # Overwrite existing foundryup
    if (-not (Test-Path $Script:FOUNDRY_BIN_DIR)) {
        New-Item -ItemType Directory -Path $Script:FOUNDRY_BIN_DIR -Force | Out-Null
    }
    Move-Item -Path $tmpFile.FullName -Destination $Script:FOUNDRY_BIN_PATH -Force

    Write-Say "successfully updated foundryup: $currentVersion ? $newVersion"
    exit 0
}

function Show-List {
    if (Test-Path $Script:FOUNDRY_VERSIONS_DIR) {
        $versions = Get-ChildItem -Path $Script:FOUNDRY_VERSIONS_DIR -Directory
        foreach ($versionDir in $versions) {
            Write-Say $versionDir.Name
            foreach ($bin in $Script:BINS) {
                $binExt = if ($Script:IsWindowsOS -or $env:OS -match "Windows") { ".exe" } else { "" }
                $binPath = Join-Path $versionDir.FullName "$bin$binExt"
                if (Test-Path $binPath) {
                    $versionOutput = & $binPath -V 2>&1
                    Write-Say "- $versionOutput"
                }
            }
            Write-Output ""
        }
    }
    else {
        foreach ($bin in $Script:BINS) {
            $binExt = if ($Script:IsWindowsOS -or $env:OS -match "Windows") { ".exe" } else { "" }
            $binPath = Join-Path $Script:FOUNDRY_BIN_DIR "$bin$binExt"
            if (Test-Path $binPath) {
                $versionOutput = & $binPath -V 2>&1
                Write-Say "- $versionOutput"
            }
        }
    }
    exit 0
}

function Use-Version {
    param([string]$VersionToUse)

    if (-not $VersionToUse) {
        Write-Err "no version provided"
    }

    $versionDir = Join-Path $Script:FOUNDRY_VERSIONS_DIR $VersionToUse
    if (-not (Test-Path $versionDir)) {
        Write-Err "version $VersionToUse not installed"
    }

    Test-BinsInUse

    if (-not (Test-Path $Script:FOUNDRY_BIN_DIR)) {
        New-Item -ItemType Directory -Path $Script:FOUNDRY_BIN_DIR -Force | Out-Null
    }

    foreach ($bin in $Script:BINS) {
        $binExt = if ($Script:IsWindowsOS -or $env:OS -match "Windows") { ".exe" } else { "" }
        $srcBinPath = Join-Path $versionDir "$bin$binExt"
        $dstBinPath = Join-Path $Script:FOUNDRY_BIN_DIR "$bin$binExt"

        if (Test-Path $srcBinPath) {
            Copy-Item -Path $srcBinPath -Destination $dstBinPath -Force
            $versionOutput = & $dstBinPath -V 2>&1
            Write-Say "use - $versionOutput"

            # Check if the default path of the binary is not in FOUNDRY_BIN_DIR
            $whichCmd = Get-Command "$bin$binExt" -ErrorAction SilentlyContinue
            $whichPath = if ($whichCmd) { $whichCmd.Source } else { $null }
            if ($whichPath -and $whichPath -ne $dstBinPath) {
                Write-Warn ""
                Write-Warning @"
There are multiple binaries with the name '$bin$binExt' present in your PATH.
This may be the result of installing '$bin' using another method,
like Cargo or other package managers.
You may need to remove '$whichPath' or move '$($Script:FOUNDRY_BIN_DIR)'
earlier in your PATH to allow the newly installed version to take precedence!

"@
            }
        }
    }
    exit 0
}

function Test-InstallerUpToDate {
    Write-Say "checking if foundryup is up to date..."

    try {
        $content = Invoke-Download -Url $Script:FOUNDRY_BIN_URL
        if ($content -match 'FOUNDRYUP_INSTALLER_VERSION="([0-9]+\.[0-9]+\.[0-9]+)"') {
            $remoteVersion = $Matches[1]
        }
        else {
            Write-Warn "Could not determine remote foundryup version. Skipping version check."
            return
        }

        if (Compare-Version -Version1 $remoteVersion -Version2 $Script:FOUNDRYUP_INSTALLER_VERSION) {
            Write-Warning @"

Your installation of foundryup is out of date.

Installed: $($Script:FOUNDRYUP_INSTALLER_VERSION) ? Latest: $remoteVersion

To update, run:

  foundryup.ps1 -Update

Updating is highly recommended as it gives you access to the latest features and bug fixes.

"@
        }
        else {
            Write-Say "foundryup is up to date."
        }
    }
    catch {
        Write-Warn "Could not check for foundryup updates. Continuing..."
    }
}

function Install-FromBinaries {
    param(
        [string]$VersionToInstall,
        [string]$RepositoryName,
        [string]$BranchName,
        [string]$CommitHash,
        [string]$PlatformOverride,
        [string]$ArchOverride
    )

    $tag = $VersionToInstall

    # Normalize versions (handle channels, versions without v prefix)
    if ($VersionToInstall -match '^nightly') {
        $VersionToInstall = "nightly"
    }
    elseif ($VersionToInstall -match '^\d') {
        # Add v prefix
        $VersionToInstall = "v$VersionToInstall"
        $tag = $VersionToInstall
    }

    Write-Say "installing foundry (version $VersionToInstall, tag $tag)"

    # Determine platform
    if ($PlatformOverride) {
        $platform = $PlatformOverride.ToLower()
    }
    else {
        if ($Script:IsWindowsOS -or $env:OS -match "Windows") {
            $platform = "win32"
        }
        elseif ($IsLinux) {
            $platform = "linux"
        }
        elseif ($IsMacOS) {
            $platform = "darwin"
        }
        else {
            $platform = "win32"  # Default to Windows
        }
    }

    $ext = if ($platform -eq "win32") { "zip" } else { "tar.gz" }

    # Determine architecture
    if ($ArchOverride) {
        $architecture = $ArchOverride.ToLower()
    }
    else {
        $arch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture
        if ($arch -eq "X64") {
            $architecture = "amd64"
        }
        elseif ($arch -eq "Arm64") {
            $architecture = "arm64"
        }
        else {
            $architecture = "amd64"
        }
    }

    # Compute URLs
    $releaseUrl = "https://github.com/$RepositoryName/releases/download/$tag/"
    $attestationUrl = "${releaseUrl}foundry_${VersionToInstall}_${platform}_${architecture}.attestation.txt"
    $binArchiveUrl = "${releaseUrl}foundry_${VersionToInstall}_${platform}_${architecture}.$ext"
    # Note: manpages not currently downloaded in Windows version
    # $manTarballUrl = "${releaseUrl}foundry_man_${VersionToInstall}.tar.gz"

    if (-not (Test-Path $Script:FOUNDRY_VERSIONS_DIR)) {
        New-Item -ItemType Directory -Path $Script:FOUNDRY_VERSIONS_DIR -Force | Out-Null
    }

    $attestationMissing = $false

    # If --force is not set, check SHA verification
    if (-not $Script:FOUNDRYUP_IGNORE_VERIFICATION) {
        Write-Say "checking if forge, cast, anvil, and chisel for $tag version are already installed"

        # Download attestation file
        $tmpDir = New-TemporaryFile | ForEach-Object { Remove-Item $_; New-Item -ItemType Directory -Path $_.FullName }
        $tmpAttestation = Join-Path $tmpDir.FullName "attestation.txt"

        try {
            Invoke-Download -Url $attestationUrl -OutFile $tmpAttestation
            $attestationContent = Get-Content -Path $tmpAttestation -Raw

            if (-not $attestationContent -or $attestationContent -match 'Not Found') {
                $attestationMissing = $true
            }
            else {
                $attestationArtifactLink = (Get-Content -Path $tmpAttestation -First 1).Trim()
                if (-not $attestationArtifactLink) {
                    $attestationMissing = $true
                }
            }
        }
        catch {
            $attestationMissing = $true
        }
        finally {
            if (Test-Path $tmpAttestation) {
                Remove-Item -Path $tmpAttestation -Force
            }
        }

        if ($attestationMissing) {
            Write-Say "no attestation found for this release, skipping SHA verification"
        }
        else {
            Write-Say "found attestation for $tag version, downloading attestation artifact, checking..."

            # Download attestation artifact
            $tmpArtifact = Join-Path $tmpDir.FullName "foundry-attestation.sigstore.json"
            Invoke-Download -Url "$attestationArtifactLink/download" -OutFile $tmpArtifact

            # Extract payload
            $artifactContent = Get-Content -Path $tmpArtifact -Raw | ConvertFrom-Json
            $payloadB64 = $artifactContent.payload
            $payloadBytes = [System.Convert]::FromBase64String($payloadB64)
            $payloadJson = [System.Text.Encoding]::UTF8.GetString($payloadBytes) | ConvertFrom-Json

            # Extract hashes
            $Script:HASH_NAMES = @()
            $Script:HASH_VALUES = @()

            foreach ($subject in $payloadJson.subject) {
                if ($subject.name -and $subject.digest.sha256) {
                    $Script:HASH_NAMES += $subject.name
                    $Script:HASH_VALUES += $subject.digest.sha256
                }
            }

            Remove-Item -Path $tmpArtifact -Force

            # Check if binaries are already installed and match expected hashes
            $versionDir = Join-Path $Script:FOUNDRY_VERSIONS_DIR $tag
            $allMatch = $true

            foreach ($bin in $Script:BINS) {
                $expected = ""
                $binExt = if ($platform -eq "win32") { ".exe" } else { "" }

                for ($i = 0; $i -lt $Script:HASH_NAMES.Count; $i++) {
                    if ($Script:HASH_NAMES[$i] -eq $bin -or $Script:HASH_NAMES[$i] -eq "$bin.exe") {
                        $expected = $Script:HASH_VALUES[$i]
                        break
                    }
                }

                $binPath = Join-Path $versionDir "$bin$binExt"

                if (-not $expected -or -not (Test-Path $binPath)) {
                    $allMatch = $false
                    break
                }

                $actual = Get-FileSha256 -FilePath $binPath
                if ($actual -ne $expected) {
                    $allMatch = $false
                    break
                }
            }

            if ($allMatch) {
                Write-Say "version $tag already installed and verified, activating..."
                Use-Version -VersionToUse $tag
                Write-Say "done!"
                exit 0
            }
        }

        Remove-Item -Path $tmpDir.FullName -Recurse -Force
        Write-Say "binaries not found or do not match expected hashes, downloading new binaries"
    }

    # Download and extract binaries
    Write-Say "downloading forge, cast, anvil, and chisel for $tag version"

    $tmpDir = New-TemporaryFile | ForEach-Object { Remove-Item $_; New-Item -ItemType Directory -Path $_.FullName }
    $tmpArchive = Join-Path $tmpDir.FullName "foundry.$ext"

    Invoke-Download -Url $binArchiveUrl -OutFile $tmpArchive

    $versionDir = Join-Path $Script:FOUNDRY_VERSIONS_DIR $tag
    if (-not (Test-Path $versionDir)) {
        New-Item -ItemType Directory -Path $versionDir -Force | Out-Null
    }

    if ($platform -eq "win32") {
        Expand-Archive -Path $tmpArchive -DestinationPath $versionDir -Force
    }
    else {
        # Use tar for non-Windows platforms
        tar -xzf $tmpArchive -C $versionDir
    }

    Remove-Item -Path $tmpDir.FullName -Recurse -Force

    # Verify downloaded binaries
    if ($Script:FOUNDRYUP_IGNORE_VERIFICATION) {
        Write-Say "skipped SHA verification for downloaded binaries due to -Force flag"
    }
    elseif ($attestationMissing) {
        Write-Say "no attestation found for these binaries, skipping SHA verification for downloaded binaries"
    }
    else {
        Write-Say "verifying downloaded binaries against the attestation file"

        $failed = $false
        foreach ($bin in $Script:BINS) {
            $expected = ""
            $binExt = if ($platform -eq "win32") { ".exe" } else { "" }

            for ($i = 0; $i -lt $Script:HASH_NAMES.Count; $i++) {
                if ($Script:HASH_NAMES[$i] -eq $bin -or $Script:HASH_NAMES[$i] -eq "$bin.exe") {
                    $expected = $Script:HASH_VALUES[$i]
                    break
                }
            }

            $binPath = Join-Path $versionDir "$bin$binExt"

            if (-not $expected) {
                Write-Say "no expected hash for $bin"
                $failed = $true
                continue
            }

            if (-not (Test-Path $binPath)) {
                Write-Say "binary $bin not found at $binPath"
                $failed = $true
                continue
            }

            $actual = Get-FileSha256 -FilePath $binPath
            if ($actual -ne $expected) {
                Write-Say "$bin hash verification failed:"
                Write-Say "  expected: $expected"
                Write-Say "  actual:   $actual"
                $failed = $true
            }
            else {
                Write-Say "$bin verified ?"
            }
        }

        if ($failed) {
            Write-Err "one or more binaries failed post-installation verification"
        }
    }

    # Use newly installed version
    Use-Version -VersionToUse $tag

    Write-Say "done!"
}

function Install-FromSource {
    param(
        [string]$LocalRepoPath,
        [string]$RepositoryName,
        [string]$BranchName,
        [string]$CommitHash,
        [string]$PRNumber
    )

    Assert-CommandExists "cargo"

    $cargoBuildArgs = @("build", "--bins", "--release")

    if ($Script:FOUNDRYUP_JOBS) {
        $cargoBuildArgs += "--jobs"
        $cargoBuildArgs += $Script:FOUNDRYUP_JOBS.ToString()
    }

    if ($LocalRepoPath) {
        # Install from local repository
        Write-Say "installing from $LocalRepoPath"

        Push-Location $LocalRepoPath
        try {
            Invoke-Ensure -ScriptBlock { cargo @cargoBuildArgs }

            foreach ($bin in $Script:BINS) {
                $binExt = if ($Script:IsWindowsOS -or $env:OS -match "Windows") { ".exe" } else { "" }
                $dstBinPath = Join-Path $Script:FOUNDRY_BIN_DIR "$bin$binExt"

                # Remove prior installations if they exist
                if (Test-Path $dstBinPath) {
                    Remove-Item -Path $dstBinPath -Force
                }

                # Copy from local repo binaries to bin dir
                $srcBinPath = Join-Path (Get-Location) "target\release\$bin$binExt"
                if (Test-Path $srcBinPath) {
                    Copy-Item -Path $srcBinPath -Destination $dstBinPath -Force
                }
            }

            Write-Say "done"
        }
        finally {
            Pop-Location
        }
        exit 0
    }

    # Install from GitHub repository
    $BranchName = if ($BranchName) { $BranchName } else { "master" }
    $repoPath = Join-Path $Script:FOUNDRY_DIR $RepositoryName
    $author = $RepositoryName.Split('/')[0]

    # Clone if doesn't exist
    if (-not (Test-Path $repoPath)) {
        $authorDir = Join-Path $Script:FOUNDRY_DIR $author
        if (-not (Test-Path $authorDir)) {
            New-Item -ItemType Directory -Path $authorDir -Force | Out-Null
        }

        Push-Location $authorDir
        try {
            Invoke-Ensure -ScriptBlock { git clone "https://github.com/$RepositoryName" }
        }
        finally {
            Pop-Location
        }
    }

    # Checkout branch/commit
    Push-Location $repoPath
    try {
        Invoke-Ensure -ScriptBlock { git fetch origin "${BranchName}:remotes/origin/${BranchName}" }
        Invoke-Ensure -ScriptBlock { git checkout "origin/${BranchName}" }

        # Determine custom version
        $customVersion = ""
        if ($CommitHash) {
            Invoke-Ensure -ScriptBlock { git checkout $CommitHash }
            $customVersion = "$author-commit-$CommitHash"
        }
        elseif ($PRNumber) {
            $customVersion = "$author-pr-$PRNumber"
        }
        else {
            $normalizedBranch = $BranchName -replace '/', '-'
            $customVersion = "$author-branch-$normalizedBranch"
        }

        Write-Say "installing version $customVersion"

        # Build the repo
        Invoke-Ensure -ScriptBlock { cargo @cargoBuildArgs }

        # Create version directory
        $versionDir = Join-Path $Script:FOUNDRY_VERSIONS_DIR $customVersion
        if (-not (Test-Path $versionDir)) {
            New-Item -ItemType Directory -Path $versionDir -Force | Out-Null
        }

        # Move binaries to version directory
        foreach ($bin in $Script:BINS) {
            $binExt = if ($Script:IsWindowsOS -or $env:OS -match "Windows") { ".exe" } else { "" }
            $srcBinPath = Join-Path (Get-Location) "target\release\$bin$binExt"

            if (Test-Path $srcBinPath) {
                Move-Item -Path $srcBinPath -Destination (Join-Path $versionDir "$bin$binExt") -Force
            }
        }

        # Use newly built version
        Pop-Location
        Use-Version -VersionToUse $customVersion

        Write-Say "done"
    }
    catch {
        Pop-Location
        throw
    }
}

#endregion

#region Main Entry Point

function Main {
    # Handle help
    if ($Help) {
        Show-Usage
        exit 0
    }

    # Handle version
    if ($Version) {
        Show-Version
    }

    # Handle update
    if ($Update) {
        Update-Foundryup
    }

    # Handle list
    if ($List) {
        Show-List
    }

    # Handle use
    if ($Use) {
        Use-Version -VersionToUse $Use
    }

    # Check for required commands
    Assert-CommandExists "git"

    # Print banner
    Show-Banner

    # Check if installer is up to date
    Test-InstallerUpToDate

    # Handle PR flag
    if ($PR -and $Branch) {
        Write-Err "cannot use -PR and -Branch at the same time"
    }

    if ($PR -and -not $Branch) {
        $Branch = "refs/pull/$PR/head"
    }

    Test-BinsInUse

    # Install from local path
    if ($Path) {
        if ($Repo -or $Branch -or $Install) {
            Write-Warn "-Branch, -Install, -Use, and -Repo arguments are ignored during local install"
        }

        Install-FromSource -LocalRepoPath $Path
    }

    $Repo = if ($Repo) { $Repo } else { "foundry-rs/foundry" }

    # Install from binaries (default for foundry-rs/foundry without branch/commit)
    if ($Repo -eq "foundry-rs/foundry" -and -not $Branch -and -not $Commit) {
        $versionToInstall = if ($Install) { $Install } else { 'stable' }

        Install-FromBinaries -VersionToInstall $versionToInstall -RepositoryName $Repo `
            -BranchName $Branch -CommitHash $Commit -PlatformOverride $Platform -ArchOverride $Arch
    }
    # Install from source
    else {
        Install-FromSource -RepositoryName $Repo -BranchName $Branch -CommitHash $Commit -PRNumber $PR
    }
}

# Run main function
Main

#endregion

