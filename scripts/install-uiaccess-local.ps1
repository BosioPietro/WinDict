<#
.SYNOPSIS
    Installs a self-signed WinDict that can show its card above Start and Search.

.DESCRIPTION
    Windows only lets an app draw above Start and Search if it runs with
    "UI access", which requires the executable to be:
      1. built with uiAccess="true" in its manifest (cargo feature `uiaccess`),
      2. signed by a certificate the machine trusts,
      3. installed in a protected folder such as C:\Program Files.

    This script does all three, from a local executable or by building one
    from this repository; install-uiaccess.ps1 does the same with the latest
    release instead. It creates a fresh self-signed code-signing
    certificate, trusts it on this machine, signs WinDict with it and then
    deletes the certificate's private key, so nothing else can ever be signed
    with it. Re-run it to update; run it with -Uninstall to remove everything.

    It also keeps a copy of install-uiaccess.ps1 beside WinDict, and
    registers a scheduled task, "WinDict Update", that runs it elevated.
    "Update app" in WinDict's tray menu starts the task, so updating doesn't
    ask for permission.

    Run from an elevated PowerShell:
        powershell -ExecutionPolicy Bypass -File scripts\install-uiaccess-local.ps1

.PARAMETER Exe
    A WinDict executable built with `--features uiaccess` (for example
    WinDict-<version>-uiaccess.exe from a release). By default the script uses
    such a file next to itself, or builds one from this repository.

.PARAMETER Updater
    The install-uiaccess.ps1 for the update task to run. By default, the one
    next to this script.

.PARAMETER Uninstall
    Removes the installed copy, its autostart entry, its certificates and its
    update task.
#>
#Requires -RunAsAdministrator
param(
    [string]$Exe,
    [string]$Updater,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
$InstallDir = Join-Path $env:ProgramFiles 'WinDict'
$Target = Join-Path $InstallDir 'WinDict.exe'
$Subject = 'CN=WinDict Local Code Signing'
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$Task = 'WinDict Update'

function Remove-WinDictCertificates {
    Get-ChildItem Cert:\LocalMachine\Root, Cert:\LocalMachine\TrustedPublisher |
        Where-Object Subject -eq $Subject |
        Remove-Item
}

# Stop a running copy so the file can be replaced.
Get-Process -Name windict -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300

if ($Uninstall) {
    $run = Get-ItemProperty $RunKey -Name WinDict -ErrorAction SilentlyContinue
    if ($run -and $run.WinDict -like "*$Target*") {
        Remove-ItemProperty $RunKey -Name WinDict
    }
    Unregister-ScheduledTask -TaskName $Task -Confirm:$false -ErrorAction SilentlyContinue
    Remove-Item $InstallDir -Recurse -Force -ErrorAction SilentlyContinue
    Remove-WinDictCertificates
    Write-Host 'WinDict (UI access) removed.'
    return
}

if (-not $Exe) {
    $bundled = Get-ChildItem $PSScriptRoot -Filter 'WinDict-*-uiaccess.exe' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime | Select-Object -Last 1
    if ($bundled) {
        $Exe = $bundled.FullName
    } else {
        $repo = Split-Path $PSScriptRoot -Parent
        Write-Host 'Building WinDict with UI access...'
        Push-Location $repo
        try {
            cargo build --release --features uiaccess
            if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
        } finally {
            Pop-Location
        }
        $Exe = Join-Path $repo 'target\release\windict.exe'
    }
}
if (-not (Test-Path $Exe)) { throw "Executable not found: $Exe" }

New-Item -ItemType Directory -Force $InstallDir | Out-Null
Copy-Item $Exe $Target -Force

# A fresh certificate for every install; old ones are removed.
Remove-WinDictCertificates
$cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject $Subject `
    -CertStoreLocation Cert:\CurrentUser\My -KeyExportPolicy NonExportable `
    -NotAfter (Get-Date).AddYears(20)
# Exported where only administrators can write, so it can't be swapped
# before it's trusted.
$cer = Join-Path $InstallDir "windict-$($cert.Thumbprint).cer"
try {
    Export-Certificate -Cert $cert -FilePath $cer | Out-Null
    Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\Root | Out-Null
    Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\TrustedPublisher | Out-Null
    $signature = Set-AuthenticodeSignature -FilePath $Target -Certificate $cert -HashAlgorithm SHA256
} finally {
    Remove-Item $cer -ErrorAction SilentlyContinue
    # Without its private key the certificate can't sign anything else.
    Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)" -DeleteKey
}
if ($signature.Status -ne 'Valid') {
    throw "Signing failed: $($signature.StatusMessage)"
}

# The update task runs the installed install-uiaccess.ps1 elevated, as this
# user, with no window: a task with the highest privileges starts without
# asking, and only administrators can change what it runs.
if (-not $Updater) { $Updater = Join-Path $PSScriptRoot 'install-uiaccess.ps1' }
if (Test-Path $Updater) {
    $script = Join-Path $InstallDir 'install-uiaccess.ps1'
    if ((Resolve-Path $Updater).Path -ne $script) { Copy-Item $Updater $script -Force }
    $powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $action = New-ScheduledTaskAction -Execute (Join-Path $env:SystemRoot 'System32\conhost.exe') `
        -Argument "--headless `"$powershell`" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$script`" -Update"
    $user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
        -ExecutionTimeLimit (New-TimeSpan -Minutes 10)
    Register-ScheduledTask -TaskName $Task -Action $action -Principal $principal -Settings $settings `
        -Description 'Installs the latest WinDict when "Update app" is chosen in its tray menu.' `
        -Force | Out-Null
} else {
    Write-Warning "install-uiaccess.ps1 isn't next to this script: WinDict won't be able to update itself."
}

# Point an existing "Start with Windows" entry at the installed copy.
if (Get-ItemProperty $RunKey -Name WinDict -ErrorAction SilentlyContinue) {
    Set-ItemProperty $RunKey -Name WinDict -Value "`"$Target`""
}

# Start it un-elevated, as the signed-in user.
Start-Process explorer.exe -ArgumentList "`"$Target`""
Write-Host "WinDict installed to $Target and started."
Write-Host 'Use "Start with Windows" in its tray menu to launch it at sign-in.'
