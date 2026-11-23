<#
Builds the Android APK and Windows ZIP on this computer with developer paths stripped, then refuses to
finish unless every file passes ci/privacy.py. Output: target/release-local (APK, ZIP, README, SHA256.json).
Run from a plain terminal or via cmd redirection; it needs about 5 GiB of free RAM for the release link.

  ./scripts/package-release.ps1 -IdentifiersFile C:\private\identifiers.json

-IdentifiersFile is an untracked JSON array of strings (account names, e-mail addresses, machine name) that
must never appear in a shipped binary. Nothing here changes machine settings, network or the phone.
#>
[CmdletBinding()]
param(
    [string]$IdentifiersFile,
    [string]$MsysRoot = 'C:\tonalli-tools\msys64',
    # Package the binaries from the last successful build without compiling (no RAM-heavy step).
    [switch]$ReuseBuilt,
    # Optional on-device AI (llama.cpp, Qwen models download in the app). Same profile as the local test builds.
    [switch]$Ai,
    [switch]$SkipAndroid,
    [switch]$SkipWindows
)
$ErrorActionPreference = 'Stop'
$repository = Split-Path $PSScriptRoot -Parent
Set-Location $repository
foreach ($path in @($repository, $env:USERPROFILE, $env:LOCALAPPDATA)) {
    if ($path -match '\\Users\\') { throw "Build through scripts/package-release-neutral.ps1: '$path' would be baked into C-compiled strings." }
}
if ($MsysRoot -match '\\Users\\' -or -not (Test-Path (Join-Path $MsysRoot 'usr/bin/perl.exe'))) {
    throw "Copy MSYS2 to a real folder outside any profile (for example robocopy <profile>\Tonalli\tools\msys64 C:\tonalli-tools\msys64 /E) and pass -MsysRoot."
}
$env:TONALLI_MSYS_ROOT = $MsysRoot
function Require-Success([string]$operation) {
    if ($LASTEXITCODE -ne 0) { throw "$operation failed (exit $LASTEXITCODE)" }
}
function Require-Memory {
    $free = (Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory
    Write-Output "Free RAM: $free KiB"
    if ($free -lt 5242880) { throw 'Close other apps first: a release build needs 5 GiB of free RAM.' }
}

# Strip developer paths from Rust, MSVC C and Go output; no debug symbols in shipped binaries.
$env:RUSTFLAGS = "--remap-path-prefix=$repository=/tonalli --remap-path-prefix=$env:USERPROFILE=/toolchain -C debuginfo=0"
$env:CARGO_PROFILE_RELEASE_DEBUG = '0'
$env:GOFLAGS = '-trimpath'

$suffix = if ($Ai) { '-AI' } else { '' }
$output = Join-Path $repository "target/release-local$(if ($Ai) { '-ai' })"
if (Test-Path $output) { Remove-Item -LiteralPath $output -Recurse -Force }
New-Item -ItemType Directory -Path $output | Out-Null
$source = (& git rev-parse --short HEAD).Trim()

if (-not $ReuseBuilt) {
    & ./android/build-web.ps1
    Require-Success 'UI build and packaging'
}

if (-not $SkipAndroid) {
    if (-not $ReuseBuilt) {
        Require-Memory
        # Clang embeds source paths in bundled C (sqlcipher/openssl); map them like Rust's.
        $env:CFLAGS_aarch64_linux_android = "-ffile-prefix-map=$repository=/tonalli -ffile-prefix-map=$env:USERPROFILE=/toolchain"
        if ($Ai) { $env:CXXFLAGS_aarch64_linux_android = $env:CFLAGS_aarch64_linux_android; & ./android/build-core.ps1 -Ai -Mesh }
        else { & ./android/build-core.ps1 -Mesh -Jobs 2 }
        Require-Success 'Android native core'
        Remove-Item Env:CFLAGS_aarch64_linux_android, Env:CXXFLAGS_aarch64_linux_android -ErrorAction SilentlyContinue
        $aiProperty = "-PenableAi=$(if ($Ai) { 'true' } else { 'false' })"
        & ./android/gradle-local.ps1 --no-daemon --max-workers=2 assembleDebug $aiProperty -PenableMesh=true -PskipCoreBuild -PskipWebBuild
        Require-Success 'APK assembly'
    }
    $verifyFlags = if ($Ai) { '-Ai -Mesh' } else { '-Mesh' }
    & cmd.exe /c "powershell.exe -NoProfile -File android\verify-apk.ps1 $verifyFlags 2>&1"
    Require-Success 'APK structure/signature/UI checks'
    Copy-Item android/app/build/outputs/apk/debug/app-debug.apk "$output/Tonalli-Android-arm64$suffix.apk"
}

if (-not $SkipWindows) {
    if (-not $ReuseBuilt) {
        Require-Memory
        $trim = "/d1trimfile:$env:USERPROFILE /d1trimfile:$repository"
        $env:CFLAGS = $trim
        $env:CXXFLAGS = $trim
        if ($Ai) {
            # llama.cpp builds through CMake; the installed VS 2019 Build Tools CMake does not know newer generators.
            # cmake-rs already adds -A x64 and -T host=x64, so only the generator is set.
            $env:CMAKE_GENERATOR = 'Visual Studio 16 2019'
            $env:CMAKE_BUILD_PARALLEL_LEVEL = '1'
        }
        try {
            $features = if ($Ai) { 'ai,mesh' } else { 'mesh' }
            $jobs = if ($Ai) { 1 } else { 2 }
            & ./scripts/with-toolchain.ps1 Native cargo build --manifest-path desktop/Cargo.toml --release --locked --features $features -j $jobs
            Require-Success 'Windows native shell'
        } finally {
            Remove-Item Env:CFLAGS, Env:CXXFLAGS, Env:CMAKE_GENERATOR, Env:CMAKE_GENERATOR_PLATFORM, Env:CMAKE_GENERATOR_TOOLSET, Env:CMAKE_BUILD_PARALLEL_LEVEL -ErrorAction SilentlyContinue
        }
    }
    if (-not (Test-Path desktop/target/release/tonalli-desktop.exe)) { throw 'No built Windows executable; build without -ReuseBuilt first.' }
    $bundle = Join-Path $output 'Windows-x64'
    # -Strip keeps the MinGW install path out of the DLL's debug info.
    & ./native/maps-extract/build.ps1 -Platform Windows -Strip -Output "$bundle/tonalli_maps.dll"
    Require-Success 'Windows country extractor'
    & python -c 'import ctypes,sys; library=ctypes.CDLL(sys.argv[1]); assert library.TonalliExtractMap' "$bundle/tonalli_maps.dll"
    Require-Success 'Country extractor load/export'
    Remove-Item -LiteralPath "$bundle/tonalli_maps.h" -ErrorAction SilentlyContinue
    Copy-Item desktop/target/release/tonalli-desktop.exe $bundle
    Copy-Item web-stub/dist "$bundle/web" -Recurse
    New-Item -ItemType Directory "$bundle/licenses" | Out-Null
    Copy-Item android/licenses/* "$bundle/licenses" -Recurse
    Copy-Item native/maps-extract/licenses "$bundle/licenses/country-extractor" -Recurse
    Copy-Item third_party/mesh-gateway-licenses "$bundle/licenses/mesh-gateway" -Recurse
    Copy-Item crates/meshtastic-fork/TONALLI-CHANGES.md "$bundle/licenses/mesh-gateway/meshtastic-0.1.9/TONALLI-CHANGES.md"
    Copy-Item LICENSE, NOTICE, LICENSES.md "$bundle/licenses"
    & ./scripts/new-portable-zip.ps1 -Folder $bundle -Zip "$output/Tonalli-Windows-x64$suffix.zip"
    Require-Success 'Windows ZIP packaging'
    if ([IO.Path]::GetFullPath($bundle) -ne [IO.Path]::GetFullPath((Join-Path $output 'Windows-x64'))) { throw 'Unexpected staging path' }
    Remove-Item -LiteralPath $bundle -Recurse -Force
}

@"
Tonalli preview, source $source
Features: native offline library, maps, notes, media, nearby sharing, unified Chat (local room, Mesh radio, broker). $(if ($Ai) { 'Includes the optional on-device AI engine (off until you load a model; models download inside the app, nothing is bundled).' } else { 'Optional AI is not included.' })
Android: ARM64, Android 14+. Install Tonalli-Android-arm64$suffix.apk. It uses the standard Android debug signer, so it updates an app installed from the same signer; never uninstall or clear data to install it.
Windows: Windows 11 x64 with WebView2. Extract the whole ZIP and run Windows-x64/tonalli-desktop.exe; keep the web, licenses and country-extractor DLL beside it. No system install needed.
These builds are tested on the developer's devices, not formally released or signed for stores.
"@ | Set-Content "$output/README.txt" -Encoding utf8
$records = @(Get-ChildItem $output -File | ForEach-Object {
    [ordered]@{ file = $_.Name; bytes = $_.Length; sha256 = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
})
$records | ConvertTo-Json -Depth 3 | Set-Content "$output/SHA256.json" -Encoding utf8

$inspect = "python ci\privacy.py --artifacts ""$output"" --explain 2>&1"
if ($IdentifiersFile) { $inspect = "python ci\privacy.py --artifacts ""$output"" --identifiers ""$IdentifiersFile"" --explain 2>&1" }
& cmd.exe /c $inspect
Require-Success 'Final artifact privacy inspection'
Write-Output "Release files are in $output"
