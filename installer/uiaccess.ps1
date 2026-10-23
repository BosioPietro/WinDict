<#
.SYNOPSIS
    Turns UI access on or off for the installed WinDict.

.DESCRIPTION
    Windows only lets an app draw above Start and Search if it runs with
    "UI access", which requires the executable to be:
      1. built with uiAccess="true" in its manifest (cargo feature `uiaccess`),
      2. signed by a certificate the machine trusts,
      3. installed in a protected folder such as C:\Program Files.

    The installer takes care of 1 and 3, putting WinDict-uiaccess.exe beside
    WinDict.exe unsigned; this script does 2. It creates a fresh self-signed
    code-signing certificate, trusts it on this machine, signs
    WinDict-uiaccess.exe with it and then deletes the certificate's private
    key, so nothing else can ever be signed with it. It records the choice as
    UIAccess in HKLM\Software\WinDict, where WinDict.exe reads it to start
    WinDict-uiaccess.exe in its place.

    Run it elevated, with one of:
      -Enable   signs WinDict-uiaccess.exe and turns UI access on.
      -Resign   signs it again if UI access is on: the installer runs this
                after every install, since an update replaces the signed file.
      -Disable  turns UI access off and removes the certificates.

    With -Restart, WinDict is then started again, so the build that runs
    is the one the choice calls for; WinDict passes it when the user turns
    UI access on or off (see src/installed.rs). If the script fails, WinDict
    keeps running, to say so.
#>
#Requires -RunAsAdministrator
param(
    [switch]$Enable,
    [switch]$Resign,
    [switch]$Disable,
    [switch]$Restart
)

$ErrorActionPreference = 'Stop'
# Runs elevated: modules come only from the system's folders, not ones the
# user can write to.
$env:PSModulePath = @(
    "$env:ProgramFiles\WindowsPowerShell\Modules"
    "$env:SystemRoot\System32\WindowsPowerShell\v1.0\Modules"
) -join ';'

$Exe = Join-Path $PSScriptRoot 'WinDict-uiaccess.exe'
$Key = 'HKLM:\Software\WinDict'
$Subject = 'CN=WinDict Local Code Signing'

function Remove-WinDictCertificates {
    Get-ChildItem Cert:\LocalMachine\Root, Cert:\LocalMachine\TrustedPublisher |
        Where-Object Subject -eq $Subject |
        Remove-Item
}

function Restart-WinDict {
    if (-not $Restart) { return }
    Get-Process -Name WinDict, WinDict-uiaccess -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 300
    # Through Explorer, so WinDict runs as the user, not elevated.
    Start-Process explorer.exe -ArgumentList "`"$(Join-Path $PSScriptRoot 'WinDict.exe')`""
}

if ($Disable) {
    Remove-ItemProperty $Key -Name UIAccess -ErrorAction SilentlyContinue
    Remove-WinDictCertificates
    Restart-WinDict
    return
}
if (-not ($Enable -or $Resign)) { throw 'Pass -Enable, -Resign or -Disable.' }
if ($Resign) {
    $on = Get-ItemProperty $Key -Name UIAccess -ErrorAction SilentlyContinue
    if (-not $on -or $on.UIAccess -ne 1) { return }
}
if (-not (Test-Path $Exe)) { throw "Executable not found: $Exe" }

# A fresh certificate every time; old ones are removed.
Remove-WinDictCertificates
$cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject $Subject `
    -CertStoreLocation Cert:\CurrentUser\My -KeyExportPolicy NonExportable `
    -NotAfter (Get-Date).AddYears(20)
# Exported where only administrators can write, so it can't be swapped
# before it's trusted.
$cer = Join-Path $PSScriptRoot "windict-$($cert.Thumbprint).cer"
try {
    Export-Certificate -Cert $cert -FilePath $cer | Out-Null
    Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\Root | Out-Null
    Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\TrustedPublisher | Out-Null
    $signature = Set-AuthenticodeSignature -FilePath $Exe -Certificate $cert -HashAlgorithm SHA256
} finally {
    Remove-Item $cer -ErrorAction SilentlyContinue
    # Without its private key the certificate can't sign anything else.
    Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)" -DeleteKey
}
if ($signature.Status -ne 'Valid') {
    Remove-WinDictCertificates
    throw "Signing failed: $($signature.StatusMessage)"
}

if (-not (Test-Path $Key)) { New-Item $Key | Out-Null }
Set-ItemProperty $Key -Name UIAccess -Value 1 -Type DWord
Restart-WinDict
