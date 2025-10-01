# `foundryup`

Update or revert to a specific Foundry branch with ease.

`foundryup` supports installing and managing multiple versions.

## Installing

### Linux / macOS / WSL

```sh
curl -L https://foundry.paradigm.xyz | bash
```

### Windows

```powershell
# Download and run install.ps1
irm https://foundry.paradigm.xyz/install.ps1 | iex

# Or if you have the repository locally:
.\install.ps1
```

## Usage

### Linux / macOS / WSL

To install the **nightly** version:

```sh
foundryup
```

### Windows

To install the **stable** version (default):

```powershell
foundryup.ps1
```

### Common Commands

#### Install a specific version

```sh
# Linux / macOS / WSL
foundryup --install nightly
```

```powershell
# Windows
foundryup.ps1 -Install nightly
```

#### List all installed versions

```sh
# Linux / macOS / WSL
foundryup --list
```

```powershell
# Windows
foundryup.ps1 -List
```

#### Switch to a specific installed version

```sh
# Linux / macOS / WSL
foundryup --use nightly-00efa0d5965269149f374ba142fb1c3c7edd6c94
```

```powershell
# Windows
foundryup.ps1 -Use nightly-00efa0d5965269149f374ba142fb1c3c7edd6c94
```

#### Install from a specific branch

```sh
# Linux / macOS / WSL
foundryup --branch release/0.1.0
```

```powershell
# Windows
foundryup.ps1 -Branch release/0.1.0
```

#### Install from a fork

```sh
# Linux / macOS / WSL
foundryup --repo transmissions11/foundry
```

```powershell
# Windows
foundryup.ps1 -Repo transmissions11/foundry
```

#### Install from a specific branch in a fork

```sh
# Linux / macOS / WSL
foundryup --repo transmissions11/foundry --branch patch-10
```

```powershell
# Windows
foundryup.ps1 -Repo transmissions11/foundry -Branch patch-10
```

#### Install from a Pull Request

```sh
# Linux / macOS / WSL
foundryup --pr 1071
```

```powershell
# Windows
foundryup.ps1 -PR 1071
```

#### Install from a specific commit

```sh
# Linux / macOS / WSL
foundryup -C 94bfdb2
```

```powershell
# Windows
foundryup.ps1 -Commit 94bfdb2
```

#### Install from a local repository

**Note**: `--branch`, `--repo`, and `--version` flags are ignored during local installations.

```sh
# Linux / macOS / WSL
foundryup --path ./git/foundry
```

```powershell
# Windows
foundryup.ps1 -Path .\git\foundry
```

---

## Uninstalling

Foundry contains everything in a `.foundry` directory, usually located in:
- Linux: `/home/<user>/.foundry/`
- macOS: `/Users/<user>/.foundry/`
- Windows: `C:\Users\<user>\.foundry`

where `<user>` is your username.

**⚠️ Warning**: The `.foundry` directory can contain keystores. Make sure to backup any keystores you want to keep before removing it.

### Remove Foundry

```sh
# Linux / macOS
rm -rf ~/.foundry
```

```powershell
# Windows
Remove-Item -Recurse -Force "$env:USERPROFILE\.foundry"
```

### Remove from PATH

#### Linux / macOS / WSL

Edit your shell configuration file (`.bashrc`, `.zshrc`, etc.) and remove the line that adds Foundry to PATH:

```sh
export PATH="$PATH:/home/user/.foundry/bin"
```

#### Windows

The installer adds Foundry to your user PATH automatically. To remove it:

1. Open **System Properties** → **Environment Variables**
2. Under **User variables**, select **Path** and click **Edit**
3. Remove the entry containing `.foundry\bin`
4. Click **OK** to save

Or use PowerShell:

```powershell
# Get current user PATH
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")

# Remove .foundry\bin from PATH
$newPath = ($userPath -split ';' | Where-Object { $_ -notlike '*foundry*' }) -join ';'
[Environment]::SetEnvironmentVariable("Path", $newPath, "User")
```
