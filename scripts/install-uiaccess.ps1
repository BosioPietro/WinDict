<#
.SYNOPSIS
    Downloads the latest WinDict release and installs it with UI access.

.DESCRIPTION
    Fetches WinDict-<version>-uiaccess.exe from the latest GitHub release and
    installs it with install-uiaccess-local.ps1, which signs it with a
    self-signed certificate trusted on this machine only and copies it to
    C:\Program Files\WinDict. No Rust toolchain is needed. Re-run it to update;
    run it with -Uninstall to remove everything.

    Run from an elevated PowerShell:
        powershell -ExecutionPolicy Bypass -File install-uiaccess.ps1

.PARAMETER Version
    A release tag such as v0.11.0 to install instead of the latest release.

.PARAMETER Uninstall
    Removes the installed copy, its autostart entry and its certificates.
#>
#Requires -RunAsAdministrator
param(
    [string]$Version,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 defaults to older TLS and a slow progress bar.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$ProgressPreference = 'SilentlyContinue'

$Api = 'https://api.github.com/repos/BosioPietro/WinDict/releases'
$release = if ($Version) {
    Invoke-RestMethod "$Api/tags/$Version"
} else {
    Invoke-RestMethod "$Api/latest"
}

$work = Join-Path $env:TEMP "windict-install-$([guid]::NewGuid())"
New-Item -ItemType Directory $work | Out-Null
try {
    function Get-ReleaseAsset([string]$Pattern) {
        $asset = $release.assets | Where-Object name -like $Pattern | Select-Object -First 1
        if (-not $asset) { throw "Release $($release.tag_name) has no asset matching $Pattern" }
        $path = Join-Path $work $asset.name
        Invoke-WebRequest $asset.browser_download_url -OutFile $path -UseBasicParsing
        $path
    }

    # Use the local installer next to this script, or the release's copy.
    $installer = Join-Path $PSScriptRoot 'install-uiaccess-local.ps1'
    if (-not ($PSScriptRoot -and (Test-Path $installer))) {
        $installer = Get-ReleaseAsset 'install-uiaccess-local.ps1'
    }

    if ($Uninstall) {
        & $installer -Uninstall
        return
    }

    Write-Host "Downloading WinDict $($release.tag_name)..."
    $exe = Get-ReleaseAsset 'WinDict-*-uiaccess.exe'
    & $installer -Exe $exe
} finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
