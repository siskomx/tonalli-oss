[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$repository = Split-Path $PSScriptRoot -Parent
$previousTarget = $env:CARGO_TARGET_DIR
$meshFreeKiB = (Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory
if ($meshFreeKiB -lt 5 * 1024 * 1024) { throw 'Mesh build needs 5 GiB free memory' }
Push-Location $repository
try {
    $env:CARGO_TARGET_DIR = Join-Path $repository 'target/mesh-android-native'
    & "$PSScriptRoot/with-toolchain.ps1" Android rtk proxy cargo ndk -t arm64-v8a -P 34 build -p tonalli-mesh-gateway --lib --features android-ble --release --locked -j 1
    if ($LASTEXITCODE -ne 0) { throw 'Mesh Android native build failed' }
    $source = Join-Path $env:CARGO_TARGET_DIR 'aarch64-linux-android/release/libtonalli_mesh_gateway.so'
    $destination = Join-Path $repository 'android/mesh-spike/build/generated/jniLibs/arm64-v8a'
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    Copy-Item -LiteralPath $source -Destination $destination
    Write-Output 'Mesh Android native library staged; assemble the optional mesh-spike module separately'
} finally {
    $env:CARGO_TARGET_DIR = $previousTarget
    Pop-Location
}
