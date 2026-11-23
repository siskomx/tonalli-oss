<#
Signs and publishes the app-update manifest for an existing GitHub release.

The release v<Version> must already hold the package files. This script hashes the local copies in
-AssetsDir, writes update-manifest.json and update-manifest.sig, and uploads both to the fixed
"update-feed" release, which is what apps follow. The private key stays outside the repository.

  scripts/publish-update.ps1 -Repo owner/repo -AssetsDir target\release-v3 -Notes notes.txt
  scripts/publish-update.ps1 -Repo owner/repo -AssetsDir target\release-v3 -DryRun
#>
param(
    [Parameter(Mandatory = $true)][string]$Repo,
    [Parameter(Mandatory = $true)][string]$AssetsDir,
    [string]$Version,
    [string]$Notes,
    [string]$Published = (Get-Date -Format 'yyyy-MM-dd'),
    [string]$KeyPath = (Join-Path $env:LOCALAPPDATA 'Tonalli\publisher-keys\update-v1.pk8'),
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)
if ($Repo -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') { throw 'Repo must look like owner/repo' }
if (-not $Version) {
    $Version = [regex]::Match((Get-Content Cargo.toml -Raw), '(?ms)\[workspace\.package\].*?^version\s*=\s*"([^"]+)"').Groups[1].Value
}
if (-not $Version) { throw 'Could not read the workspace version; pass -Version' }
if (-not (Test-Path -LiteralPath $KeyPath)) { throw "Update signing key not found: $KeyPath" }

$assets = @(
    @{ Platform = 'android-arm64'; File = 'Tonalli-Android-arm64.apk' },
    @{ Platform = 'windows-x64'; File = 'Tonalli-Windows-x64.zip' },
    @{ Platform = 'ios-arm64'; File = 'Tonalli-iOS-arm64.ipa' }
) | Where-Object { Test-Path -LiteralPath (Join-Path $AssetsDir $_.File) }
if ($assets.Count -eq 0) { throw "No package files found in $AssetsDir" }

$out = Join-Path ([System.IO.Path]::GetTempPath()) ("tonalli-update-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $out | Out-Null
try {
    $arguments = @('run', '-q', '-p', 'spike-cli', '--', 'update-manifest',
        '--version', $Version, '--published', $Published,
        '--page', "https://github.com/$Repo/releases/tag/v$Version",
        '--base-url', "https://github.com/$Repo/releases/download/v$Version/",
        '--key', $KeyPath, '--out-dir', $out)
    if ($Notes) { $arguments += @('--notes-file', $Notes) }
    foreach ($asset in $assets) { $arguments += @('--asset', "$($asset.Platform)=$(Join-Path $AssetsDir $asset.File)") }
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) { throw 'Signing the update manifest failed' }
    Get-Content (Join-Path $out 'update-manifest.json') -Raw
    if ($DryRun) { 'Dry run: nothing was uploaded.'; return }

    $tags = @(& gh release list --repo $Repo --limit 200 --json tagName --jq '.[].tagName')
    if ($LASTEXITCODE -ne 0) { throw 'Could not list releases' }
    if ($tags -notcontains 'update-feed') {
        & gh release create update-feed --repo $Repo --prerelease --title 'Update feed' --notes 'Signed update manifest that Tonalli apps check when update checks are turned on. Not an app download.'
        if ($LASTEXITCODE -ne 0) { throw 'Could not create the update-feed release' }
    }
    & gh release upload update-feed --repo $Repo --clobber (Join-Path $out 'update-manifest.json') (Join-Path $out 'update-manifest.sig')
    if ($LASTEXITCODE -ne 0) { throw 'Uploading the update manifest failed' }
    "Published update manifest for v$Version to $Repo (update-feed)."
} finally {
    Remove-Item -LiteralPath $out -Recurse -Force -ErrorAction SilentlyContinue
}
