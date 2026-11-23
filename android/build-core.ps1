[CmdletBinding()]
param([ValidateRange(1, 16)][int]$Jobs = 2, [switch]$Ai, [switch]$Mesh, [switch]$DryRun)

$ErrorActionPreference = 'Stop'
$repository = Split-Path $PSScriptRoot -Parent
$previousTarget = $env:CARGO_TARGET_DIR
$cmakeNames = @('CMAKE_GENERATOR', 'CMAKE_GENERATOR_aarch64-linux-android', 'CMAKE_MAKE_PROGRAM',
    'CMAKE_BUILD_PARALLEL_LEVEL', 'CMAKE_GENERATOR_PLATFORM', 'CMAKE_GENERATOR_TOOLSET', 'CMAKE_GENERATOR_INSTANCE')
$previousCmake = @{}
foreach ($name in $cmakeNames) { $previousCmake[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
function Get-BuildCmakeEnvironment {
    $snapshot = [ordered]@{}
    foreach ($name in $cmakeNames) { $snapshot[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
    return $snapshot
}
$features = if ($Ai) { 'ffi,ai' } else { 'ffi' }
if ($Mesh) { $features += ',mesh-android' }
if ($Ai) { $Jobs = 1 }
Push-Location $repository
try {
    $env:CARGO_TARGET_DIR = Join-Path $repository 'target/android-native'
    $sdk = if ($env:TONALLI_CI_TOOLS -eq '1') { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android/Sdk' }
    if ($Ai) {
        $env:CMAKE_GENERATOR = 'Ninja'
        # cmake-rs prefers the exact target override over TARGET_ and global values.
        [Environment]::SetEnvironmentVariable('CMAKE_GENERATOR_aarch64-linux-android', 'Ninja', 'Process')
        $env:CMAKE_BUILD_PARALLEL_LEVEL = '1'
        $env:CMAKE_MAKE_PROGRAM = (Join-Path $sdk 'cmake/3.31.6/bin/ninja.exe').Replace('\', '/')
        foreach ($name in @('CMAKE_GENERATOR_PLATFORM', 'CMAKE_GENERATOR_TOOLSET', 'CMAKE_GENERATOR_INSTANCE')) {
            [Environment]::SetEnvironmentVariable($name, $null, 'Process')
        }
        if (-not (Test-Path -LiteralPath $env:CMAKE_MAKE_PROGRAM)) { throw "Missing Android AI Ninja: $env:CMAKE_MAKE_PROGRAM" }
    }
    # UniFFI needs ELF .symtab; override stripping for the final crate only.
    $androidArguments = @('Android', 'rtk', 'cargo', 'ndk', '-t', 'arm64-v8a', '-P', '34', 'rustc', '-p', 'tonalli-core', '--lib', '--features', $features, '--release', '--locked', '-j', "$Jobs", '--', '-C', 'strip=none')
    $nativeLibrary = Join-Path $env:CARGO_TARGET_DIR 'aarch64-linux-android/release/libtonalli_core.so'
    $generatedRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'app/build/generated'))
    if ($Ai) { $generatedRoot = Join-Path $generatedRoot 'ai' }
    if ($Mesh) { $generatedRoot = if ($Ai) { Join-Path $PSScriptRoot 'app/build/generated/ai-mesh' } else { Join-Path $PSScriptRoot 'app/build/generated/mesh' } }
    $kotlin = Join-Path $generatedRoot 'uniffi'
    $jni = Join-Path $generatedRoot 'jniLibs/arm64-v8a'
    $bindgenArguments = @('Native', 'rtk', 'cargo', 'run', '-p', 'tonalli-core', '--features', 'bindgen', '--bin', 'uniffi-bindgen', '--locked', '-j', "$Jobs", '--', 'generate', '--library', $nativeLibrary, '--language', 'kotlin', '--config', 'crates/core/uniffi.toml', '--out-dir', $kotlin)
    if ($DryRun) {
        $androidEnvironment = Get-BuildCmakeEnvironment
        foreach ($name in $cmakeNames) { [Environment]::SetEnvironmentVariable($name, $previousCmake[$name], 'Process') }
        $env:CARGO_TARGET_DIR = Join-Path $repository 'target'
        [ordered]@{ features = $features; jobs = $Jobs
            outputs = @{ profile = $features; generatedRoot = $generatedRoot; kotlin = $kotlin; jni = $jni }
            android = @{ arguments = $androidArguments; environment = $androidEnvironment }
            bindgen = @{ arguments = $bindgenArguments; environment = (Get-BuildCmakeEnvironment) }
            executed = $false } | ConvertTo-Json -Depth 5
        return
    }
    & ./scripts/with-toolchain.ps1 @androidArguments
    if ($LASTEXITCODE -ne 0) { throw 'Rust Android build failed' }
    New-Item -ItemType Directory -Force $kotlin, $jni | Out-Null
    # Android CMake settings must not leak into the host binding generator.
    foreach ($name in $cmakeNames) { [Environment]::SetEnvironmentVariable($name, $previousCmake[$name], 'Process') }
    $env:CARGO_TARGET_DIR = Join-Path $repository 'target'
    & ./scripts/with-toolchain.ps1 @bindgenArguments
    if ($LASTEXITCODE -ne 0) { throw 'UniFFI binding generation failed' }
    Copy-Item -LiteralPath $nativeLibrary -Destination $jni
    $strip = Join-Path $sdk 'ndk/29.0.14206865/toolchains/llvm/prebuilt/windows-x86_64/bin/llvm-strip.exe'
    & $strip --strip-all "$jni/libtonalli_core.so"
    if ($LASTEXITCODE -ne 0) { throw 'Stripping the packaged release library failed' }
    & ./native/maps-extract/build.ps1 -Platform Android -Output "$jni/libtonalli_maps.so"
    if ($LASTEXITCODE -ne 0) { throw 'Android country extractor build failed' }
    & $strip --strip-all "$jni/libtonalli_maps.so"
    if ($LASTEXITCODE -ne 0) { throw 'Stripping country extractor failed' }
} finally {
    foreach ($name in $cmakeNames) { [Environment]::SetEnvironmentVariable($name, $previousCmake[$name], 'Process') }
    $env:CARGO_TARGET_DIR = $previousTarget
    Pop-Location
}
