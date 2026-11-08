# Builds Superteem Voice and installs it for the current user, with a Start Menu shortcut.
# Run with -Uninstall to remove the app. Recordings in Documents\Superteem Voice are always kept.
param([switch]$Uninstall)
$ErrorActionPreference = 'Stop'

$installDir = Join-Path $env:LOCALAPPDATA 'Programs\Superteem Voice'
$shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Superteem Voice.lnk'

if ($Uninstall) {
    Remove-Item $shortcut -Force -ErrorAction SilentlyContinue
    Remove-Item $installDir -Recurse -Force -ErrorAction SilentlyContinue
    Write-Output 'Superteem Voice removed. Recordings were kept.'
    return
}

. (Join-Path $PSScriptRoot 'build-env.ps1')
$manifest = Join-Path $PSScriptRoot 'Cargo.toml'
cargo build --release --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { throw 'Build failed' }

New-Item -ItemType Directory -Force $installDir | Out-Null
$exe = Join-Path $installDir 'superteem-voice.exe'
Copy-Item (Join-Path $PSScriptRoot 'target\release\superteem-voice.exe') $exe -Force

$link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
$link.TargetPath = $exe
$link.WorkingDirectory = $installDir
$link.Save()
Write-Output "Installed to $exe. Open it from the Start Menu: Superteem Voice."
