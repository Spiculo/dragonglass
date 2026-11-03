# Build the release binaries on Windows.
#
# GPUI compiles its DirectX shaders with `fxc.exe` from the Windows SDK. The SDK is
# normally installed with the Visual Studio C++ workload, but its `bin` directory is not
# always on PATH, so locate the newest x64 copy and prepend it before invoking cargo.
#
# Usage: powershell -ExecutionPolicy Bypass -File packaging\windows\build.ps1

$ErrorActionPreference = "Stop"

$kits = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
if (Test-Path $kits) {
    $fxc = Get-ChildItem $kits -Directory |
        Sort-Object Name -Descending |
        ForEach-Object {
            Get-ChildItem $_.FullName -Recurse -Filter fxc.exe -ErrorAction SilentlyContinue |
                Where-Object { $_.FullName -match "\\x64\\fxc\.exe$" }
        } |
        Select-Object -First 1

    if ($fxc) {
        $env:PATH = "$($fxc.DirectoryName);$env:PATH"
        Write-Host "Using $($fxc.FullName)"
    } else {
        Write-Warning "fxc.exe was not found. Install the Windows SDK 'Desktop development with C++' workload."
    }
} else {
    Write-Warning "Windows Kits\10\bin was not found. Install the Windows SDK."
}

cargo build --release -p dragonglass-ui -p dragonglass-cli
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

Write-Host ""
Write-Host "Built:"
Write-Host "  target\release\dragon-glass-app.exe"
Write-Host "  target\release\dragon-glass.exe"
