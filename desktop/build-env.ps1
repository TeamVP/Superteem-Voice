# Prepares the build environment for Superteem Voice. Dot-source it: . .\build-env.ps1 [-Android]
# whisper.cpp needs CMake and libclang, and its build script needs one fix to cross-compile from Windows.
param([switch]$Android)
$ErrorActionPreference = 'Stop'

$vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'

# libclang generates the whisper.cpp bindings. The Python wheel is the smallest way to get it.
if (-not $env:LIBCLANG_PATH) {
    $tools = Join-Path $env:LOCALAPPDATA 'Superteem Voice\build-tools\libclang'
    $dll = Get-ChildItem -Recurse -Filter libclang.dll $tools -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $dll) {
        New-Item -ItemType Directory -Force $tools | Out-Null
        python -m pip download libclang --no-deps --only-binary=:all: -d $tools
        if ($LASTEXITCODE -ne 0) { throw 'Could not download libclang' }
        $zip = Join-Path $tools 'libclang.zip'
        Copy-Item (Get-ChildItem $tools -Filter *.whl | Select-Object -First 1).FullName $zip -Force
        Expand-Archive $zip (Join-Path $tools 'x') -Force
        Remove-Item $zip
        $dll = Get-ChildItem -Recurse -Filter libclang.dll $tools | Select-Object -First 1
    }
    $env:LIBCLANG_PATH = $dll.DirectoryName
}

# whisper-rs-sys adds the MSVC-only /utf-8 flag whenever the build machine is Windows, which breaks the
# Android compiler. A local copy with that check fixed is used through [patch] in Cargo.toml.
$vendor = Join-Path $PSScriptRoot 'vendor\whisper-rs-sys'
if (-not (Test-Path (Join-Path $vendor 'build.rs'))) {
    $registry = "$env:USERPROFILE\.cargo\registry\src\*\whisper-rs-sys-0.15.0"
    if (-not (Test-Path $registry)) {
        # The app's own manifest can't fetch it while the patch path is missing, so a scratch crate does.
        $scratch = Join-Path $env:TEMP 'stv-fetch-whisper'
        New-Item -ItemType Directory -Force (Join-Path $scratch 'src') | Out-Null
        Set-Content (Join-Path $scratch 'src\lib.rs') ''
        Set-Content (Join-Path $scratch 'Cargo.toml') "[package]`nname = `"fetch`"`nversion = `"0.0.0`"`nedition = `"2021`"`n[dependencies]`nwhisper-rs-sys = `"=0.15.0`""
        cargo fetch --manifest-path (Join-Path $scratch 'Cargo.toml')
        if ($LASTEXITCODE -ne 0) { throw 'Could not fetch whisper-rs-sys' }
    }
    $source = Get-ChildItem $registry -Directory | Select-Object -First 1
    if (-not $source) { throw 'whisper-rs-sys 0.15.0 is not in the cargo registry' }
    New-Item -ItemType Directory -Force (Split-Path $vendor) | Out-Null
    Copy-Item $source.FullName $vendor -Recurse
    $build = Join-Path $vendor 'build.rs'
    $text = Get-Content $build -Raw
    $fixed = $text.Replace("if cfg!(target_os = `"windows`") {`n        config.cxxflag(`"/utf-8`");", "if target.contains(`"msvc`") {`n        config.cxxflag(`"/utf-8`");")
    if ($fixed -eq $text) { throw 'The whisper-rs-sys build script changed; update build-env.ps1' }
    Set-Content $build $fixed -NoNewline
}

# The build script passes GGML_* variables to whisper.cpp's CMake but doesn't rebuild when they change, so
# a copy built for the wrong processor would be kept. This makes cargo rebuild it when they change.
$build = Join-Path $vendor 'build.rs'
$text = Get-Content $build -Raw
if (-not $text.Contains('rerun-if-env-changed=GGML_NATIVE')) {
    $marker = '    // Allow passing any WHISPER or CMAKE compile flags'
    if (-not $text.Contains($marker)) { throw 'The whisper-rs-sys build script changed; update build-env.ps1' }
    $watch = ('GGML_NATIVE', 'GGML_AVX', 'GGML_AVX2', 'GGML_FMA', 'GGML_F16C', 'GGML_AVX512') |
        ForEach-Object { "    println!(`"cargo:rerun-if-env-changed=$_`");`n" }
    Set-Content $build $text.Replace($marker, ($watch -join '') + $marker) -NoNewline
}

# llama.cpp (the local language model for analysis templates) brings its own, newer ggml, and two copies
# in one program are duplicate symbols. whisper.cpp is built against llama.cpp's ggml instead: llama-cpp-sys-2
# installs it as a CMake package and tells crates that depend on it where (DEP_LLAMA_GGML_CMAKE_DIR).
$llamaSys = '=0.1.161'
$manifest = Join-Path $vendor 'Cargo.toml'
$text = Get-Content $manifest -Raw
if (-not $text.Contains('[dependencies.llama-cpp-sys-2]')) {
    $text += "`n# Superteem: build against llama.cpp's ggml, so the app has one copy.`n[dependencies.llama-cpp-sys-2]`nversion = `"$llamaSys`"`ndefault-features = false`n"
    Set-Content $manifest $text -NoNewline
}
$wrapper = Join-Path $vendor 'wrapper.h'
$text = Get-Content $wrapper -Raw
if ($text.Contains('<ggml/include/ggml.h>')) {
    Set-Content $wrapper $text.Replace('<ggml/include/ggml.h>', '<ggml.h>') -NoNewline
}
$text = Get-Content $build -Raw
if (-not $text.Contains('DEP_LLAMA_GGML_CMAKE_DIR')) {
    $edits = @(
        @('        let bindings = bindings
            .clang_arg("-I./whisper.cpp/")',
          '        let ggml_include = env::var("DEP_LLAMA_GGML_CMAKE_DIR")
            .map(|d| PathBuf::from(d).join("..").join("..").join("include").display().to_string())
            .unwrap_or_else(|_| "./whisper.cpp/ggml/include".into());
        let bindings = bindings
            .clang_arg("-I./whisper.cpp/")'),
        @('            .clang_arg("-I./whisper.cpp/ggml/include")', '            .clang_arg(format!("-I{ggml_include}"))'),
        @('    let destination = config.build();',
          '    // Superteem: one ggml in the app, llama.cpp''s.
    let system_ggml = env::var("DEP_LLAMA_GGML_CMAKE_DIR").ok();
    if let Some(dir) = &system_ggml {
        config.define("WHISPER_USE_SYSTEM_GGML", "ON");
        config.define("CMAKE_PREFIX_PATH", dir);
        // The Android NDK''s toolchain only searches its sysroot for packages; naming the folder works anywhere.
        config.define("ggml_DIR", PathBuf::from(dir).join("ggml"));
    }

    let destination = config.build();'),
        @('        println!("cargo:rustc-link-lib=static=whisper");
        println!("cargo:rustc-link-lib=static=ggml");
        println!("cargo:rustc-link-lib=static=ggml-base");
        println!("cargo:rustc-link-lib=static=ggml-cpu");',
          '        println!("cargo:rustc-link-lib=static=whisper");
        if system_ggml.is_none() {
            println!("cargo:rustc-link-lib=static=ggml");
            println!("cargo:rustc-link-lib=static=ggml-base");
            println!("cargo:rustc-link-lib=static=ggml-cpu");
        }')
    )
    foreach ($e in $edits) {
        $from = $e[0].Replace("`r`n", "`n")
        if (-not $text.Contains($from)) { throw 'The whisper-rs-sys build script changed; update build-env.ps1' }
        $text = $text.Replace($from, $e[1].Replace("`r`n", "`n"))
    }
    Set-Content $build $text -NoNewline
}

if ($Android) {
    # Android Studio's SDK, unless ANDROID_HOME already points at one (as on GitHub's runners).
    $sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
    $ndk = Get-ChildItem (Join-Path $sdk 'ndk') -Directory | Sort-Object Name | Select-Object -Last 1
    if (-not $ndk) { throw 'Install the Android NDK from Android Studio' }
    $cmakeDir = Get-ChildItem (Join-Path $sdk 'cmake') -Directory | Sort-Object Name | Select-Object -Last 1
    $env:ANDROID_HOME = $sdk
    $env:NDK_HOME = $ndk.FullName
    $env:ANDROID_NDK = $ndk.FullName
    $env:ANDROID_NDK_HOME = $ndk.FullName
    if (-not $env:JAVA_HOME) { $env:JAVA_HOME = 'C:\Program Files\Android\Android Studio\jbr' }
    $env:PATH = "$(Join-Path $cmakeDir.FullName 'bin');$env:PATH"
    $env:CMAKE_GENERATOR = 'Ninja'
    $llvm = Join-Path $ndk.FullName 'toolchains\llvm\prebuilt\windows-x86_64'
    $sysroot = (Join-Path $llvm 'sysroot').Replace('\', '/')
    # Compiler headers such as stdbool.h come with the NDK's clang, not the sysroot.
    $builtins = (Get-ChildItem (Join-Path $llvm 'lib\clang') -Directory | Sort-Object Name | Select-Object -Last 1).FullName
    $builtins = (Join-Path $builtins 'include').Replace('\', '/')
    foreach ($t in @(@('aarch64_linux_android', 'aarch64-linux-android'), @('x86_64_linux_android', 'x86_64-linux-android'))) {
        Set-Item "env:BINDGEN_EXTRA_CLANG_ARGS_$($t[0])" "--target=$($t[1])29 --sysroot=$sysroot -isystem $builtins"
    }
} else {
    if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
        # The newest Visual Studio's CMake, so it knows the generator for the newest compiler.
        $found = & $vswhere -sort -products * -find 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe' | Select-Object -First 1
        if (-not $found) { throw 'CMake is required to build Superteem Voice' }
        $env:CMAKE = $found
    }
    # The libclang wheel has no compiler headers, so bindgen uses the ones from Visual Studio and the Windows SDK.
    $stdbool = & $vswhere -products * -find 'VC\Tools\MSVC\*\include\stdbool.h' | Select-Object -Last 1
    $ucrt = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\Include' | Where-Object { Test-Path "$($_.FullName)\ucrt\stdio.h" } | Sort-Object Name | Select-Object -Last 1
    if (-not $stdbool -or -not $ucrt) { throw 'Visual Studio C++ tools and the Windows SDK are required' }
    $env:BINDGEN_EXTRA_CLANG_ARGS = "-I`"$(Split-Path $stdbool)`" -I`"$($ucrt.FullName)\ucrt`""
    # whisper.cpp is otherwise built for the build machine's own processor. GitHub's machines have AVX-512,
    # which most PCs (Intel 12th gen and later desktops included) don't, so speech to text crashed the app
    # there with an illegal instruction. AVX2, from 2013 on, runs on practically every Windows PC in use.
    $env:GGML_NATIVE = 'OFF'
    $env:GGML_AVX = 'ON'
    $env:GGML_AVX2 = 'ON'
    $env:GGML_FMA = 'ON'
    $env:GGML_F16C = 'ON'
    $env:GGML_AVX512 = 'OFF'
}
