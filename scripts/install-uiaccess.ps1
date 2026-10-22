<#
.SYNOPSIS
    Downloads the latest WinDict release and installs it with UI access.

.DESCRIPTION
    Fetches WinDict-<version>-uiaccess.exe from the latest GitHub release and
    installs it with install-uiaccess-local.ps1, which signs it with a
    self-signed certificate trusted on this machine only and copies it to
    C:\Program Files\WinDict. No Rust toolchain is needed. Re-run it to update;
    run it with -Uninstall to remove everything.

    The installer keeps a copy of this script in C:\Program Files\WinDict and
    registers a scheduled task that runs it with -Update. "Update app" in
    WinDict's tray menu starts that task.

    Run from an elevated PowerShell:
        powershell -ExecutionPolicy Bypass -File install-uiaccess.ps1

.PARAMETER Version
    A release tag such as v0.11.0 to install instead of the latest release.

.PARAMETER Uninstall
    Removes the installed copy, its autostart entry, its certificates and its
    update task.

.PARAMETER Update
    What the update task runs: installs the latest release with the
    release's own installer, and starts WinDict again even if that fails.
#>
#Requires -RunAsAdministrator
param(
    [string]$Version,
    [switch]$Uninstall,
    [switch]$Update
)

$ErrorActionPreference = 'Stop'
# The update task runs this elevated, and any app of the user can start it:
# modules come only from the system's folders, not ones the user can write to.
$env:PSModulePath = @(
    "$env:ProgramFiles\WindowsPowerShell\Modules"
    "$env:SystemRoot\System32\WindowsPowerShell\v1.0\Modules"
) -join ';'
# Windows PowerShell 5.1 defaults to older TLS and a slow progress bar.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$ProgressPreference = 'SilentlyContinue'

$Api = 'https://api.github.com/repos/BosioPietro/WinDict/releases'
$InstallDir = Join-Path $env:ProgramFiles 'WinDict'
# Downloads go where only administrators can write, so nothing can swap
# them before they run elevated.
$work = Join-Path $InstallDir ".download-$([guid]::NewGuid())"
try {
    $release = if ($Version) {
        Invoke-RestMethod "$Api/tags/$Version"
    } else {
        Invoke-RestMethod "$Api/latest"
    }
    New-Item -ItemType Directory -Force $work | Out-Null

    function Get-ReleaseAsset([string]$Pattern) {
        $asset = $release.assets | Where-Object name -like $Pattern | Select-Object -First 1
        if (-not $asset) { throw "Release $($release.tag_name) has no asset matching $Pattern" }
        $path = Join-Path $work $asset.name
        Invoke-WebRequest $asset.browser_download_url -OutFile $path -UseBasicParsing
        # GitHub publishes the SHA-256 of each file.
        if ($asset.digest -match '^sha256:([0-9a-fA-F]{64})$') {
            if ((Get-FileHash $path -Algorithm SHA256).Hash -ne $Matches[1]) {
                throw "$($asset.name) was damaged in the download"
            }
        } elseif ($Update) {
            throw "Release $($release.tag_name) has no digest for $($asset.name)"
        }
        $path
    }

    # Use the local installer next to this script, or the release's copy.
    # An update always takes the release's, and the release's copy of this
    # script for the next one.
    $installer = Join-Path $PSScriptRoot 'install-uiaccess-local.ps1'
    $updater = $PSCommandPath
    if ($Update -or -not ($PSScriptRoot -and (Test-Path $installer))) {
        $installer = Get-ReleaseAsset 'install-uiaccess-local.ps1'
    }
    if ($Update) {
        $updater = Get-ReleaseAsset 'install-uiaccess.ps1'
    }

    if ($Uninstall) {
        & $installer -Uninstall
        return
    }

    Write-Host "Downloading WinDict $($release.tag_name)..."
    $exe = Get-ReleaseAsset 'WinDict-*-uiaccess.exe'
    & $installer -Exe $exe -Updater $updater
} catch {
    if (-not $Update) { throw }
    # Nobody sees an update fail: at least WinDict comes back.
    $target = Join-Path $InstallDir 'WinDict.exe'
    if ((Test-Path $target) -and -not (Get-Process -Name windict -ErrorAction SilentlyContinue)) {
        Start-Process explorer.exe -ArgumentList "`"$target`""
    }
} finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
