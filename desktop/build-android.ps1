# Builds the Android app as an APK, ready to install with adb. -Install also installs it on the connected phone or emulator.
# It is signed with the release key when STV_KEYSTORE (a .jks path), STV_KEYSTORE_PASSWORD, STV_KEY_ALIAS and
# STV_KEY_PASSWORD are set, as in the release workflow, and with the Android debug key otherwise.
param([switch]$Install, [string[]]$Targets = @('aarch64', 'x86_64'))
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'build-env.ps1') -Android

Push-Location $PSScriptRoot
try {
    # The Gradle build calls back into the CLI through npm, so it must be a project dependency.
    if (-not (Test-Path 'node_modules\.bin\tauri.cmd')) { npm install --no-audit --no-fund }
    npm run tauri -- android build --apk --target @Targets
    if ($LASTEXITCODE -ne 0) { throw 'Android build failed' }
} finally {
    Pop-Location
}

# Release APKs come out unsigned. The debug key lets them install for testing; published builds use the release key,
# and every release must use the same one or phones refuse the update.
$unsigned = Get-ChildItem -Recurse (Join-Path $PSScriptRoot 'gen\android\app\build\outputs\apk') -Filter '*-unsigned.apk' |
    Sort-Object LastWriteTime | Select-Object -Last 1
$buildTools = Get-ChildItem (Join-Path $env:ANDROID_HOME 'build-tools') -Directory | Sort-Object Name | Select-Object -Last 1
$apk = Join-Path $PSScriptRoot 'target\superteem-voice.apk'
New-Item -ItemType Directory -Force (Split-Path $apk) | Out-Null
& (Join-Path $buildTools.FullName 'zipalign.exe') -f -p 4 $unsigned.FullName $apk
if ($env:STV_KEYSTORE) {
    & (Join-Path $buildTools.FullName 'apksigner.bat') sign --ks $env:STV_KEYSTORE --ks-pass env:STV_KEYSTORE_PASSWORD `
        --ks-key-alias $env:STV_KEY_ALIAS --key-pass env:STV_KEY_PASSWORD $apk
} else {
    $keystore = Join-Path $env:USERPROFILE '.android\debug.keystore'
    if (-not (Test-Path $keystore)) {
        & (Join-Path $env:JAVA_HOME 'bin\keytool.exe') -genkeypair -keystore $keystore -storepass android -alias androiddebugkey `
            -keypass android -keyalg RSA -validity 10000 -dname 'CN=Android Debug,O=Android,C=US'
    }
    & (Join-Path $buildTools.FullName 'apksigner.bat') sign --ks $keystore --ks-pass pass:android $apk
}
if ($LASTEXITCODE -ne 0) { throw 'Signing failed' }
Write-Output "Built $apk"

if ($Install) {
    & (Join-Path $env:ANDROID_HOME 'platform-tools\adb.exe') install -r $apk
}
