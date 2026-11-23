$ErrorActionPreference = 'Stop'
$parameters = (Get-Command -Name "$PSScriptRoot/build-core.ps1").Parameters
if (-not $parameters.ContainsKey('DryRun') -or -not $parameters.ContainsKey('Ai')) {
    throw 'Build profile must declare Ai and DryRun before this harness can invoke it'
}
$names = @('CARGO_TARGET_DIR', 'CMAKE_GENERATOR', 'CMAKE_GENERATOR_aarch64-linux-android',
    'CMAKE_MAKE_PROGRAM', 'CMAKE_BUILD_PARALLEL_LEVEL', 'CMAKE_GENERATOR_PLATFORM',
    'CMAKE_GENERATOR_TOOLSET', 'CMAKE_GENERATOR_INSTANCE', 'LOCALAPPDATA')
$saved = @{}
foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
$location = (Get-Location).Path
$opensslBefore = [Environment]::GetEnvironmentVariable('OPENSSL_DIR', 'Process')
$artifactPaths = @('app/build/generated/jniLibs/arm64-v8a/libtonalli_core.so',
    'app/build/generated/uniffi/uniffi/tonalli_core/tonalli_core.kt',
    'app/build/generated/ai/jniLibs/arm64-v8a/libtonalli_core.so',
    'app/build/generated/ai/uniffi/uniffi/tonalli_core/tonalli_core.kt', 'app/build/outputs/apk/debug/app-debug.apk')
$artifactsBefore = @{}
foreach ($path in $artifactPaths) {
    $file = Get-Item -LiteralPath (Join-Path $PSScriptRoot $path) -ErrorAction SilentlyContinue
    $artifactsBefore[$path] = if ($file) { "$($file.Length):$($file.LastWriteTimeUtc.Ticks)" } else { 'absent' }
}
function Check([bool]$condition, [string]$message) { if (-not $condition) { throw $message } }
try {
    $env:CARGO_TARGET_DIR = Join-Path $PSScriptRoot '../target/build-core-profile-test/unused-target'
    $env:CMAKE_GENERATOR = 'Visual Studio 16 2019'
    [Environment]::SetEnvironmentVariable('CMAKE_GENERATOR_aarch64-linux-android', 'Visual Studio 16 2019', 'Process')
    $env:CMAKE_MAKE_PROGRAM = 'C:/build-profile-test/MSBuild.exe'
    $env:CMAKE_BUILD_PARALLEL_LEVEL = '9'
    $env:CMAKE_GENERATOR_PLATFORM = 'x64'
    $env:CMAKE_GENERATOR_TOOLSET = 'v142'
    $env:CMAKE_GENERATOR_INSTANCE = 'C:/build-profile-test/VS'
    $expected = @{}
    foreach ($name in $names) { $expected[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
    $normal = (& "$PSScriptRoot/build-core.ps1" -DryRun) | ConvertFrom-Json
    Check ($normal.features -eq 'ffi' -and $normal.jobs -eq 2) 'Default build must remain ffi-only with its existing job default'
    Check ($normal.android.arguments[$normal.android.arguments.IndexOf('--features') + 1] -eq 'ffi') 'Default actual Cargo arguments enable AI'
    Check ($normal.android.environment.CMAKE_GENERATOR -eq 'Visual Studio 16 2019') 'Normal profile changed caller CMake settings'
    Check ($normal.outputs.profile -eq 'ffi' -and $normal.outputs.generatedRoot -eq [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'app/build/generated'))) 'Default generated output profile changed'
    $ai = (& "$PSScriptRoot/build-core.ps1" -Ai -Jobs 4 -DryRun) | ConvertFrom-Json
    Check ($ai.features -eq 'ffi,ai' -and $ai.jobs -eq 1) 'AI profile must opt in and force serial Cargo jobs'
    Check ($ai.outputs.profile -eq 'ffi,ai' -and $ai.outputs.generatedRoot -eq [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'app/build/generated/ai'))) 'AI generated outputs are not isolated'
    Check ($ai.outputs.kotlin -ne $normal.outputs.kotlin -and $ai.outputs.jni -ne $normal.outputs.jni) 'Profiles share generated JNI/Kotlin output paths'
    Check ($ai.bindgen.arguments[$ai.bindgen.arguments.IndexOf('--out-dir') + 1] -eq $ai.outputs.kotlin) 'AI bindgen does not use selected isolated output'
    Check ($ai.android.arguments[$ai.android.arguments.IndexOf('--features') + 1] -eq 'ffi,ai') 'AI Cargo features missing'
    Check ($ai.android.arguments[$ai.android.arguments.IndexOf('-j') + 1] -eq '1') 'AI Cargo job cap missing'
    Check ($ai.android.environment.CMAKE_GENERATOR -eq 'Ninja') 'AI Android generator is not Ninja'
    Check ($ai.android.environment.'CMAKE_GENERATOR_aarch64-linux-android' -eq 'Ninja') 'Target-specific Visual Studio generator leaked into Android'
    Check ($ai.android.environment.CMAKE_BUILD_PARALLEL_LEVEL -eq '1') 'AI CMake concurrency is unbounded'
    Check ($ai.android.environment.CMAKE_MAKE_PROGRAM -like '*/Android/Sdk/cmake/3.31.6/bin/ninja.exe') 'AI must use the prepared Android Ninja'
    foreach ($name in @('CMAKE_GENERATOR_PLATFORM', 'CMAKE_GENERATOR_TOOLSET', 'CMAKE_GENERATOR_INSTANCE')) {
        Check ($null -eq $ai.android.environment.$name) "Visual Studio-only Android setting leaked: $name"
    }
    Check ($ai.bindgen.environment.CMAKE_GENERATOR -eq 'Visual Studio 16 2019') 'Android generator leaked into host bindgen'
    Check ($ai.bindgen.arguments[$ai.bindgen.arguments.IndexOf('--features') + 1] -eq 'bindgen') 'Host binding generator must not enable native AI'
    Check ([Environment]::GetEnvironmentVariable('OPENSSL_DIR', 'Process') -ceq $opensslBefore) 'Build profile changed the OpenSSL environment'
    foreach ($name in $names) {
        Check ([Environment]::GetEnvironmentVariable($name, 'Process') -ceq $expected[$name]) "Successful dry-run changed process variable $name"
    }
    $env:LOCALAPPDATA = Join-Path $PSScriptRoot 'missing-test-sdk'
    $failed = $false
    try { & "$PSScriptRoot/build-core.ps1" -Ai -DryRun | Out-Null } catch { $failed = $_.Exception.Message -like 'Missing Android AI Ninja:*' }
    Check $failed 'Missing Ninja must fail before any build executes'
    foreach ($name in $names | Where-Object { $_ -ne 'LOCALAPPDATA' }) {
        Check ([Environment]::GetEnvironmentVariable($name, 'Process') -ceq $expected[$name]) "Failed profile changed process variable $name"
    }
    Check ((Get-Location).Path -eq $location) 'Build profile failed to restore working directory'
    foreach ($path in $artifactPaths) {
        $file = Get-Item -LiteralPath (Join-Path $PSScriptRoot $path) -ErrorAction SilentlyContinue
        $after = if ($file) { "$($file.Length):$($file.LastWriteTimeUtc.Ticks)" } else { 'absent' }
        Check ($after -ceq $artifactsBefore[$path]) "Dry-run changed generated/installable artifact $path"
    }
    Write-Output 'PASS: actual default/AI argument planning, serial Android Ninja isolation, host/OpenSSL preservation, success/failure environment restoration; no build invoked'
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
}
