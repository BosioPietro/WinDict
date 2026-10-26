<#
.SYNOPSIS
    Updates the installed WinDict to the latest release.

.DESCRIPTION
    The installer registers a scheduled task, "WinDict Update", that runs this
    script elevated, as the user who installed WinDict, with no window: a task
    with the highest privileges starts without asking, and only
    administrators can change what it runs. "Update app" in WinDict's tray
    menu starts the task, so updating doesn't ask for permission.

    The script downloads the latest release's setup, checks it against the
    SHA-256 GitHub publishes for it and runs it silently. The setup closes
    WinDict, signs it again if UI access is on, and starts it again.

.PARAMETER Register
    Registers the task, to run this script. The installer runs this.

.PARAMETER Unregister
    Removes the task. The uninstaller runs this.
#>
#Requires -RunAsAdministrator
param(
    [switch]$Register,
    [switch]$Unregister
)

$ErrorActionPreference = 'Stop'
# Any app of the user can start the task: modules come only from the
# system's folders, not ones the user can write to. Those come from
# Windows, not from environment variables, which the user can change.
$System = [Environment]::SystemDirectory
$env:PSModulePath = @(
    Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'WindowsPowerShell\Modules'
    Join-Path $System 'WindowsPowerShell\v1.0\Modules'
) -join ';'
$Explorer = Join-Path ([Environment]::GetFolderPath('Windows')) 'explorer.exe'
$Task = 'WinDict Update'

if ($Unregister) {
    Unregister-ScheduledTask -TaskName $Task -Confirm:$false -ErrorAction SilentlyContinue
    return
}
if ($Register) {
    $powershell = Join-Path $System 'WindowsPowerShell\v1.0\powershell.exe'
    $action = New-ScheduledTaskAction -Execute (Join-Path $System 'conhost.exe') `
        -Argument "--headless `"$powershell`" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$PSCommandPath`""
    $user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
        -ExecutionTimeLimit (New-TimeSpan -Minutes 10)
    Register-ScheduledTask -TaskName $Task -Action $action -Principal $principal -Settings $settings `
        -Description 'Installs the latest WinDict when "Update app" is chosen in its tray menu.' `
        -Force | Out-Null
    return
}

# Windows PowerShell 5.1 defaults to older TLS and a slow progress bar.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$ProgressPreference = 'SilentlyContinue'

$Api = 'https://api.github.com/repos/BosioPietro/WinDict/releases/latest'
# What the setup wrote when it installed WinDict: only administrators can
# change it. The AppId is installer/windict.iss's.
$Uninstall = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{DDA83B7B-331F-48D2-BD93-4B1945B33E32}_is1'

# "v1.2.3" or "1.2.3" as a version, or $null.
function ConvertTo-Version([string]$Tag) {
    if ($Tag -match '^v?(\d+)\.(\d+)\.(\d+)$') {
        return [version]::new([int]$Matches[1], [int]$Matches[2], [int]$Matches[3])
    }
    return $null
}

# The download goes where only administrators can write, so nothing can swap
# it before it runs elevated.
$work = Join-Path $PSScriptRoot ".download-$([guid]::NewGuid())"
try {
    $release = Invoke-RestMethod $Api
    # Any app of the user can start the task, but only to update: never to
    # put back an older release, or this one again.
    $latest = ConvertTo-Version $release.tag_name
    if (-not $latest) { throw "Release $($release.tag_name) isn't a version" }
    $installed = ConvertTo-Version (Get-ItemProperty $Uninstall -Name DisplayVersion).DisplayVersion
    if (-not $installed -or $latest -le $installed) { throw "WinDict $installed is up to date" }
    $name = "WinDict-$($release.tag_name)-setup.exe"
    $asset = $release.assets | Where-Object name -eq $name | Select-Object -First 1
    if (-not $asset) { throw "Release $($release.tag_name) has no $name" }
    # GitHub publishes the SHA-256 of each file.
    if ($asset.digest -match '^sha256:([0-9a-fA-F]{64})$') {
        $sha256 = $Matches[1]
    } else {
        throw "Release $($release.tag_name) has no digest for $name"
    }
    New-Item -ItemType Directory $work | Out-Null
    $setup = Join-Path $work $name
    Invoke-WebRequest $asset.browser_download_url -OutFile $setup -UseBasicParsing
    if ((Get-FileHash $setup -Algorithm SHA256).Hash -ne $sha256) {
        throw "$name was damaged in the download"
    }
    $process = Start-Process $setup -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' `
        -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "$name failed ($($process.ExitCode))" }
} catch {
    # Nobody sees an update fail: at least WinDict comes back.
    $target = Join-Path $PSScriptRoot 'WinDict.exe'
    $running = Get-Process -Name WinDict, WinDict-uiaccess -ErrorAction SilentlyContinue
    if ((Test-Path $target) -and -not $running) {
        Start-Process $Explorer -ArgumentList "`"$target`""
    }
} finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
