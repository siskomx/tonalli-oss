param([ValidateSet('Windows','Android')][string]$Platform = 'Windows', [string]$Output, [switch]$Strip)
$ErrorActionPreference = 'Stop'
$saved = @{}
$names = @('GOOS','GOARCH','CGO_ENABLED','CC','CGO_CFLAGS','CGO_LDFLAGS')
foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
Push-Location $PSScriptRoot
try {
    $env:CGO_ENABLED = '1'
    $buildArguments = @()
    if ($Platform -eq 'Android') {
        $env:GOOS = 'android'; $env:GOARCH = 'arm64'
        $sdk = if ($env:TONALLI_CI_TOOLS -eq '1') { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android/Sdk' }
        $ndk = Join-Path $sdk 'ndk/29.0.14206865/toolchains/llvm/prebuilt/windows-x86_64/bin'
        $env:CC = "$ndk/clang.exe"
        $env:CGO_CFLAGS = '--target=aarch64-linux-android34'
        $env:CGO_LDFLAGS = '--target=aarch64-linux-android34 -Wl,-z,max-page-size=16384'
        $goRoot = (& go env GOROOT).Trim()
        if ((& go env GOVERSION).Trim() -ne 'go1.25.2') { throw 'Recheck Android CPU overlay against the pinned Go version' }
        $original = Join-Path $goRoot 'src/internal/cpu/cpu_arm64_hwcap.go'
        $patched = Join-Path $PSScriptRoot '_android-runtime/cpu_arm64_hwcap.go'
        $provenance = Get-Content (Join-Path $PSScriptRoot '_android-runtime/UPSTREAM.json') -Raw | ConvertFrom-Json
        $hasher = [Security.Cryptography.SHA256]::Create()
        try {
            $originalHash = [BitConverter]::ToString($hasher.ComputeHash([IO.File]::ReadAllBytes($original))).Replace('-', '').ToLowerInvariant()
            $patchedHash = [BitConverter]::ToString($hasher.ComputeHash([IO.File]::ReadAllBytes($patched))).Replace('-', '').ToLowerInvariant()
        } finally { $hasher.Dispose() }
        if ($originalHash -ne $provenance.original_sha256 -or $patchedHash -ne $provenance.adapted_sha256) { throw 'Go Android overlay source mismatch' }
        $overlay = Join-Path $PSScriptRoot '../../target/maps-extract/android-overlay.json'
        New-Item -ItemType Directory -Force (Split-Path $overlay) | Out-Null
        $replacements = @{}; $replacements[$original] = $patched
        @{Replace=$replacements} | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath $overlay -Encoding ascii
        $buildArguments += "-overlay=$overlay"
        if (-not $Output) { $Output = '../../target/maps-extract/arm64-v8a/libtonalli_maps.so' }
    } else {
        $env:GOOS = 'windows'; $env:GOARCH = 'amd64'
        $env:CC = Join-Path $env:USERPROFILE '.codex/tools/tonalli/strawberry-perl-5.42.3.1/c/bin/gcc.exe'
        if ($env:TONALLI_CI_TOOLS -eq '1') { $env:CC = (Get-Command gcc.exe -ErrorAction Stop).Source }
        # -Strip: no debug info, which otherwise records the compiler's install paths for a release.
        $env:CGO_CFLAGS = if ($Strip) { '-g0 -O2' } else { '' }; $env:CGO_LDFLAGS = ''
        if (-not $Output) { $Output = '../../target/maps-extract/windows-x64/tonalli_maps.dll' }
    }
    if (-not [IO.Path]::IsPathRooted($Output)) { $Output = Join-Path $PSScriptRoot $Output }
    $Output = [IO.Path]::GetFullPath($Output)
    New-Item -ItemType Directory -Force (Split-Path $Output) | Out-Null
    if ($env:TONALLI_CI_TOOLS -eq '1') { & go build @buildArguments -trimpath '-ldflags=-s -w' -buildmode=c-shared -o $Output . }
    elseif ($Strip) { & rtk proxy go build @buildArguments -trimpath '-ldflags=-s -w' -buildmode=c-shared -o $Output . }
    else { & rtk proxy go build @buildArguments -trimpath -buildmode=c-shared -o $Output . }
    if ($LASTEXITCODE -ne 0) { throw 'Country extractor build failed' }
    if (-not (Test-Path -LiteralPath $Output)) { throw 'Country extractor output missing' }
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    Pop-Location
}
