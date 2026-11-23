param([Parameter(Mandatory)][ValidateSet('Android', 'Windows')][string]$Platform)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'This driver is for an isolated hosted CI checkout.' }
$env:TONALLI_CI_TOOLS = '1'
$repository = Split-Path $PSScriptRoot -Parent
Set-Location $repository
function Require-Success([string]$operation) {
    if ($LASTEXITCODE -ne 0) { throw "$operation failed (exit $LASTEXITCODE)" }
}
function Require-Memory {
    $memory = Get-CimInstance Win32_OperatingSystem
    $free = $memory.FreePhysicalMemory
    Write-Output "Hosted CI memory: available $free / total $($memory.TotalVisibleMemorySize) KiB (one build job)"
    # Dedicated disposable VM, no user apps or AI compilation. The development
    # PC's separate 5 GiB admission rule is unchanged.
    if ($free -lt 3145728) { throw 'Hosted single-job compilation needs 3 GiB free RAM.' }
}

# No developer paths or debugging symbols belong in downloadable binaries.
$env:RUSTFLAGS = "--remap-path-prefix=$repository=/tonalli --remap-path-prefix=$env:USERPROFILE=/toolchain -C debuginfo=0"
$env:CARGO_PROFILE_RELEASE_DEBUG = '0'
$env:CARGO_BUILD_JOBS = '1'
$env:GOFLAGS = '-trimpath'

& npm.cmd --prefix web-stub ci
Require-Success 'UI dependencies'
& python scripts/prepare-map-assets.py
Require-Success 'Pinned offline map assets'
& ./android/build-web.ps1
Require-Success 'UI build and packaging'

$output = Join-Path $repository 'artifacts'
New-Item -ItemType Directory -Path $output | Out-Null
if ($Platform -eq 'Android') {
    & rustup target add aarch64-linux-android
    Require-Success 'Android Rust target'
    & cargo install cargo-ndk --version 4.1.2 --locked
    Require-Success 'cargo-ndk'
    Require-Memory
    & ./android/build-core.ps1 -Mesh -Jobs 1
    Require-Success 'Android native core'
    Require-Memory
    & ./android/gradle-local.ps1 --no-daemon --max-workers=1 assembleDebug -PenableMesh=true -PskipCoreBuild -PskipWebBuild
    Require-Success 'APK assembly'
    & powershell.exe -NoProfile -File android/verify-apk.ps1 -Mesh
    Require-Success 'APK structure/signature/UI checks'
    Copy-Item android/app/build/outputs/apk/debug/app-debug.apk "$output/Tonalli-Android-arm64-test.apk"
} else {
    Require-Memory
    # Rust's path remapping does not reach bundled C sources (for example liblzma's __FILE__ strings);
    # MSVC cl strips these prefixes itself. Scoped to this build so the Go/cgo step is unaffected.
    $trim = "/d1trimfile:$env:USERPROFILE /d1trimfile:$repository"
    $env:CFLAGS = $trim
    $env:CXXFLAGS = $trim
    try {
        & ./scripts/with-toolchain.ps1 Native cargo build --manifest-path desktop/Cargo.toml --release --locked --features mesh -j 1
        Require-Success 'Windows native shell'
    } finally {
        Remove-Item Env:CFLAGS, Env:CXXFLAGS -ErrorAction SilentlyContinue
    }
    & ./native/maps-extract/build.ps1 -Platform Windows -Output "$output/Windows-x64/tonalli_maps.dll"
    Require-Success 'Windows country extractor'
    $bundle = Join-Path $output 'Windows-x64'
    & python -c 'import ctypes,sys; library=ctypes.CDLL(sys.argv[1]); assert library.TonalliExtractMap' "$bundle/tonalli_maps.dll"
    Require-Success 'Country extractor load/export'
    Remove-Item -LiteralPath "$bundle/tonalli_maps.h"
    Copy-Item desktop/target/release/tonalli-desktop.exe $bundle
    Copy-Item web-stub/dist "$bundle/web" -Recurse
    New-Item -ItemType Directory "$bundle/licenses" | Out-Null
    Copy-Item android/licenses/* "$bundle/licenses" -Recurse
    Copy-Item native/maps-extract/licenses "$bundle/licenses/country-extractor" -Recurse
    Copy-Item third_party/mesh-gateway-licenses "$bundle/licenses/mesh-gateway" -Recurse
    Copy-Item crates/meshtastic-fork/TONALLI-CHANGES.md "$bundle/licenses/mesh-gateway/meshtastic-0.1.9/TONALLI-CHANGES.md"
    Copy-Item LICENSE,NOTICE,LICENSES.md "$bundle/licenses"
    Compress-Archive -Path $bundle -DestinationPath "$output/Tonalli-Windows-x64-test.zip"
    # Upload only the ZIP, not a second unpacked bundle.
    $resolved = [IO.Path]::GetFullPath($bundle)
    if ($resolved -ne [IO.Path]::GetFullPath((Join-Path $repository 'artifacts/Windows-x64'))) { throw 'Unexpected staging path' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
@"
Tonalli CI test build, source $env:GITHUB_SHA
Features: native non-AI app and Mesh. AI code/data and index worker are unchanged.
These artifacts pass build/structure/privacy checks, not physical-device acceptance.
Android: ARM64, Android 14+. This APK has an ephemeral CI debug signer and cannot
update an existing differently signed installation. Use a separate test device;
do not uninstall an existing app or erase its data to install this build.
Windows: extract the whole ZIP and run Windows-x64/tonalli-desktop.exe. Keep the
web, licenses and country-extractor DLL beside the EXE. No system install needed.
"@ | Set-Content "$output/README.txt" -Encoding utf8
$records = @(Get-ChildItem $output -File | ForEach-Object {
    [ordered]@{file=$_.Name; bytes=$_.Length; sha256=(Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
})
$records | ConvertTo-Json -Depth 3 | Set-Content "$output/SHA256.json" -Encoding utf8
# --explain prints only masked path shapes to stderr; cmd merges it so PowerShell does not treat it as an error.
& cmd.exe /c "python ci\privacy.py --artifacts ""$output"" --explain 2>&1"
Require-Success 'Final artifact privacy inspection'
