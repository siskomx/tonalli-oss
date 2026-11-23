<#
Run a command with the local preflight tools, without changing machine settings.
Example: ./scripts/with-toolchain.ps1 Android rtk cargo ndk -t arm64-v8a -P 34 build -p tonalli-core
#>
param(
    [ValidateSet('Native', 'Android')]
    [string]$Mode = 'Native'
)

$ErrorActionPreference = 'Stop'
$Command = @($args)
$ciTools = $env:TONALLI_CI_TOOLS -eq '1'
if ($ciTools -and $Command.Count -gt 1 -and $Command[0] -eq 'rtk') {
    $Command = $Command[1..($Command.Count - 1)]
    if ($Command[0] -eq 'proxy' -and $Command.Count -gt 1) { $Command = $Command[1..($Command.Count - 1)] }
}
if ($Command.Count -eq 0) { throw 'Pass Native or Android, followed by the command to run.' }
$savedEnvironment = @{}
$names = @('PATH', 'LIBCLANG_PATH', 'ANDROID_HOME', 'ANDROID_SDK_ROOT', 'ANDROID_NDK_HOME', 'JAVA_HOME', 'MAKEFLAGS')
foreach ($name in $names) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

$commandExitCode = 1
try {
    $sdk = Join-Path $env:LOCALAPPDATA 'Android/Sdk'
    $llvm = Join-Path $env:USERPROFILE '.codex/tools/tonalli/llvm-21.1.8/bin'
    $nasm = Join-Path $env:USERPROFILE '.codex/tools/tonalli/nasm-3.01'
    if ($ciTools) {
        $sdk = $env:ANDROID_HOME
        $llvm = Split-Path (Get-Command clang.exe -ErrorAction Stop).Source
        $nasm = Split-Path (Get-Command nasm.exe -ErrorAction Stop).Source
        if (-not (Test-Path (Join-Path $llvm 'libclang.dll'))) { throw 'CI LLVM installation must include libclang.dll' }
    }
    $env:LIBCLANG_PATH = $llvm

    if ($Mode -eq 'Android') {
        $msys = Join-Path $env:LOCALAPPDATA 'Tonalli/tools/msys64/usr/bin'
        # Release builds point this at a copy outside any user profile: MSYS bakes its real install path into OpenSSL.
        if ($env:TONALLI_MSYS_ROOT) { $msys = Join-Path $env:TONALLI_MSYS_ROOT 'usr/bin' }
        $cmake = Join-Path $sdk 'cmake/3.31.6/bin'
        if ($ciTools) {
            $msys = 'C:/msys64/usr/bin'
            $cmake = Split-Path (Get-Command cmake.exe -ErrorAction Stop).Source
        }
        $env:ANDROID_HOME = $sdk
        $env:ANDROID_SDK_ROOT = $sdk
        $env:ANDROID_NDK_HOME = Join-Path $sdk 'ndk/29.0.14206865'
        if (-not $ciTools) { $env:JAVA_HOME = Join-Path $env:ProgramFiles 'Android/Android Studio/jbr' }
        foreach ($required in @("$msys/make.exe", "$msys/perl.exe", "$cmake/cmake.exe", "$env:ANDROID_NDK_HOME/source.properties", "$env:JAVA_HOME/bin/java.exe")) {
            if (-not (Test-Path -LiteralPath $required)) { throw "Missing preflight tool: $required" }
        }
        $env:PATH = "$cmake;$msys;$sdk/platform-tools;$nasm;$llvm;$env:PATH"
        # cargo-ndk emits backslash paths; POSIX make needs slash paths for OpenSSL.
        $ndkBin = (Join-Path $env:ANDROID_NDK_HOME 'toolchains/llvm/prebuilt/windows-x86_64/bin').Replace('\', '/')
        $env:MAKEFLAGS = "$env:MAKEFLAGS CC=$ndkBin/clang.exe AR=$ndkBin/llvm-ar.exe RANLIB=$ndkBin/llvm-ranlib.exe"
    } else {
        $cmake = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/2019/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin'
        $nativePerl = Join-Path $env:USERPROFILE '.codex/tools/tonalli/strawberry-perl-5.42.3.1'
        $nativePerlBin = "$nativePerl/perl/bin"
        $nativeCBin = "$nativePerl/c/bin"
        if ($ciTools) {
            $cmake = Split-Path (Get-Command cmake.exe -ErrorAction Stop).Source
            $nativePerlBin = Split-Path (Get-Command perl.exe -ErrorAction Stop).Source
            $nativeCBin = Split-Path (Get-Command gcc.exe -ErrorAction Stop).Source
            if ((& "$nativePerlBin/perl.exe" -e 'print $^O') -ne 'MSWin32') { throw 'Native CI requires Windows Perl, not MSYS Perl' }
        }
        foreach ($required in @("$cmake/cmake.exe", "$nasm/nasm.exe", "$llvm/libclang.dll", "$nativePerlBin/perl.exe")) {
            if (-not (Test-Path -LiteralPath $required)) { throw "Missing preflight tool: $required" }
        }
        # Cargo discovers installed MSVC itself. Keep MSYS Perl out of native builds.
        $env:PATH = "$nativePerlBin;$nativeCBin;$nasm;$llvm;$cmake;$env:PATH"
    }

    $commandArguments = @()
    if ($Command.Count -gt 1) { $commandArguments = $Command[1..($Command.Count - 1)] }
    & $Command[0] @commandArguments
    $commandSucceeded = $?
    $commandExitCode = $LASTEXITCODE
    if (-not $commandSucceeded -and $commandExitCode -eq 0) { $commandExitCode = 1 }
} finally {
    foreach ($name in $names) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
    }
}

exit $commandExitCode
