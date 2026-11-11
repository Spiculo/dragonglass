#!/usr/bin/env bash
# Build the Rust core for Android, then generate the Kotlin bindings.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$root"

sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
if [[ ! -d "$sdk" ]]; then
  echo "Android SDK not found. Set ANDROID_HOME." >&2
  exit 1
fi

ndk="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [[ -z "$ndk" ]]; then
  ndk="$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -n1 || true)"
fi
if [[ -z "$ndk" ]]; then
  echo "Android NDK not found. Install one, e.g. sdkmanager 'ndk;27.2.12479018'." >&2
  exit 1
fi

export ANDROID_HOME="$sdk"
export ANDROID_NDK_HOME="$ndk"
echo "Using NDK $ndk"

cargo build -p dragonglass-ffi

case "$(uname -s)" in
  Darwin) host_lib="target/debug/libdragonglass_ffi.dylib" ;;
  *) host_lib="target/debug/libdragonglass_ffi.so" ;;
esac

if [[ ! -f "$host_lib" ]]; then
  echo "Host FFI library was not produced at $host_lib" >&2
  exit 1
fi

cargo run -q -p dragonglass-ffi-bindgen -- generate \
  --library "$host_lib" \
  --language kotlin \
  --no-format \
  --out-dir mobile/android/app/src/main/java

cargo ndk -t arm64-v8a -t x86_64 \
  -o mobile/android/app/src/main/jniLibs \
  build --release -p dragonglass-ffi

echo "Native libraries written to mobile/android/app/src/main/jniLibs"
echo "Build the app with: cd mobile/android && ./gradlew :app:assembleDebug"
