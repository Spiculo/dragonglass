# Build the Rust core for Android, then generate the Kotlin bindings.
#
# Run from anywhere: the script locates the repository root and the Android SDK.
# Requires: Rust Android targets, cargo-ndk, and an Android NDK.

$ErrorActionPreference = "Continue"

$root = Resolve-Path (Join-Path $PSScriptRoot "../../..")
Set-Location $root

$sdk = $env:ANDROID_HOME
if (-not $sdk) { $sdk = $env:ANDROID_SDK_ROOT }
if (-not $sdk) { $sdk = Join-Path $env:LOCALAPPDATA "Android\Sdk" }
if (-not (Test-Path $sdk)) {
    throw "Android SDK not found. Set ANDROID_HOME or install Android Studio."
}

$ndk = $env:ANDROID_NDK_HOME
if (-not $ndk) { $ndk = $env:ANDROID_NDK_ROOT }
if (-not $ndk) {
    $ndkRoot = Join-Path $sdk "ndk"
    if (Test-Path $ndkRoot) {
        $ndk = Get-ChildItem $ndkRoot -Directory |
            Sort-Object Name -Descending |
            Select-Object -First 1 -ExpandProperty FullName
    }
}
if (-not $ndk) {
    throw "Android NDK not found. Install one, for example: sdkmanager `"ndk;27.2.12479018`""
}

$env:ANDROID_HOME = $sdk
$env:ANDROID_NDK_HOME = $ndk
Write-Host "Using NDK $ndk"

cargo build -p dragonglass-ffi
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$hostLib = Join-Path $root "target/debug/dragonglass_ffi.dll"
if (-not (Test-Path $hostLib)) {
    throw "Host FFI library was not produced at $hostLib"
}

cargo run -q -p dragonglass-ffi-bindgen -- generate `
    --library $hostLib `
    --language kotlin `
    --no-format `
    --out-dir (Join-Path $root "mobile/android/app/src/main/java")
if ($LASTEXITCODE -ne 0) { throw "binding generation failed" }

cargo ndk -t arm64-v8a -t x86_64 `
    -o mobile/android/app/src/main/jniLibs `
    build --release -p dragonglass-ffi
if ($LASTEXITCODE -ne 0) { throw "Android native build failed" }

Write-Host "Native libraries written to mobile/android/app/src/main/jniLibs"
Write-Host "Build the app with: cd mobile/android; .\gradlew.bat :app:assembleDebug"
