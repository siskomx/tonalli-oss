param(
    [Parameter(Mandatory = $true)][int]$WaitPid,
    [Parameter(Mandatory = $true)][string]$Source,
    [Parameter(Mandatory = $true)][string]$Target,
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$Work,
    [string]$AppArgs = ''
)
# Replaces the installed application files with a verified, already unpacked update, then starts the
# application again. A failed copy restores the previous version. Only the given paths are touched.
$ErrorActionPreference = 'Stop'
$log = Join-Path $Work 'last-apply.txt'
$backup = Join-Path $Work 'backup'
$relaunch = $true
$quiet = @('/R:3', '/W:1', '/NFL', '/NDL', '/NJH', '/NJS', '/NP')
try {
    $running = Get-Process -Id $WaitPid -ErrorAction SilentlyContinue
    if ($running -and -not $running.WaitForExit(60000)) {
        $relaunch = $false
        throw 'The application did not close in time; nothing was changed.'
    }
    if (-not (Test-Path -LiteralPath $Source -PathType Container)) { throw 'The unpacked update is missing; nothing was changed.' }
    if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Recurse -Force }
    & robocopy $Target $backup /E @quiet | Out-Null
    if ($LASTEXITCODE -ge 8) { throw 'The safety copy of the current version failed; nothing was changed.' }
    # Files at the top are copied over; each folder the package ships is mirrored, so files the new
    # version dropped (old hashed web assets) do not pile up. Other folders and files are left alone.
    $copied = $true
    & robocopy $Source $Target /LEV:1 /IS /IT @quiet | Out-Null
    if ($LASTEXITCODE -ge 8) { $copied = $false }
    foreach ($folder in Get-ChildItem -LiteralPath $Source -Directory) {
        if (-not $copied) { break }
        & robocopy $folder.FullName (Join-Path $Target $folder.Name) /MIR /IS /IT @quiet | Out-Null
        if ($LASTEXITCODE -ge 8) { $copied = $false }
    }
    if (-not $copied) {
        & robocopy $backup $Target /MIR @quiet | Out-Null
        throw 'Copying the update failed; the previous version was restored.'
    }
    Remove-Item -LiteralPath $Source -Recurse -Force
    Remove-Item -LiteralPath $backup -Recurse -Force
    Set-Content -LiteralPath $log -Value 'ok' -Encoding ASCII
} catch {
    Set-Content -LiteralPath $log -Value ('failed: ' + $_.Exception.Message) -Encoding ASCII
}
if ($relaunch) {
    if ($AppArgs) { Start-Process -FilePath $Exe -WorkingDirectory $Target -ArgumentList $AppArgs }
    else { Start-Process -FilePath $Exe -WorkingDirectory $Target }
}
