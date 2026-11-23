[CmdletBinding()]
param([string]$NodeRuntime, [string]$Output)
$ErrorActionPreference = 'Stop'
function Get-Sha256([string]$File) {
    $hash = [Security.Cryptography.SHA256]::Create()
    $stream = [IO.File]::OpenRead($File)
    try { return [BitConverter]::ToString($hash.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
    finally { $stream.Dispose(); $hash.Dispose() }
}
$repository = Split-Path $PSScriptRoot -Parent
if (-not $NodeRuntime) { $NodeRuntime = (Get-Command node.exe).Source }
if (-not $Output) { $Output = Join-Path $repository 'target/deliveries/tonalli-chat-broker-windows' }
$Output = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $Output) { throw 'Choose a new output directory; existing deliveries are retained.' }
if ((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory -lt 5242880) { throw 'Need at least 5 GiB free before the native helper build.' }
Push-Location $repository
try {
    & rtk proxy powershell -NoProfile -File scripts/with-toolchain.ps1 Native rtk cargo build --release --locked -p tonalli-chat-store -j 1
    if ($LASTEXITCODE -ne 0) { throw 'Chat history helper build failed' }
    $version = & $NodeRuntime --version
    if ($LASTEXITCODE -ne 0 -or $version -notmatch '^v(2[2-9]|[3-9][0-9])\.') { throw 'Node22 or later is required.' }
    $runtimeRecord = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'licenses/node-runtime.json') -Raw | ConvertFrom-Json
    $runtimeLicense = Join-Path $PSScriptRoot "licenses/Node-v$($runtimeRecord.version)-LICENSE.txt"
    if ($version -ne "v$($runtimeRecord.version)" -or (Get-Sha256 $NodeRuntime) -ne $runtimeRecord.node_sha256) { throw 'Runtime differs from the recorded official binary.' }
    if ((Get-Sha256 $runtimeLicense) -ne $runtimeRecord.license_sha256) { throw 'Runtime license differs from its recorded source.' }
    if (-not (Test-Path -LiteralPath 'web-stub/dist/index.html')) { throw 'Build the matching shared UI first.' }
    New-Item -ItemType Directory -Path $Output | Out-Null
    foreach ($name in @('src','licenses','node_modules','package.json','package-lock.json','README.md','Start Chat.cmd')) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination $Output -Recurse
    }
    New-Item -ItemType Directory -Path (Join-Path $Output 'bin') | Out-Null
    Copy-Item -LiteralPath 'target/release/tonalli-chat-store.exe' -Destination (Join-Path $Output 'bin/tonalli-chat-store.exe')
    Copy-Item -LiteralPath 'web-stub/dist' -Destination (Join-Path $Output 'public') -Recurse
    Copy-Item -LiteralPath $NodeRuntime -Destination (Join-Path $Output 'node.exe')
    Copy-Item -LiteralPath $runtimeLicense -Destination (Join-Path $Output 'licenses/Node-LICENSE.txt')
    Copy-Item -LiteralPath 'android/licenses' -Destination (Join-Path $Output 'licenses/native') -Recurse
    foreach ($name in @('LICENSE','NOTICE','LICENSES.md')) { Copy-Item -LiteralPath $name -Destination $Output }
    [ordered]@{ node=$version; node_sha256=(Get-Sha256 $NodeRuntime); source=(& rtk git rev-parse HEAD); acceptance='pending; source package only' } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Output 'BUILD.json') -Encoding UTF8
    Compress-Archive -LiteralPath $Output -DestinationPath "$Output.zip" -CompressionLevel Optimal
    $zipHash = Get-Sha256 "$Output.zip"
    Write-Output "SHA256 $zipHash  $Output.zip"
} finally { Pop-Location }
