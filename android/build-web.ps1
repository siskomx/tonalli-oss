$ErrorActionPreference = 'Stop'
$repository = Split-Path $PSScriptRoot -Parent
Push-Location $repository
try {
    & npm.cmd --prefix web-stub run build
    if ($LASTEXITCODE -ne 0) { throw 'Shared UI build failed' }
    & node web-stub/scripts/package-android-ui.mjs
    if ($LASTEXITCODE -ne 0) { throw 'Android UI packaging failed' }
} finally { Pop-Location }
