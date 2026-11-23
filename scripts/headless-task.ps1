<#
Manage Tonalli's window-less LAN sharing as a per-user Scheduled Task. No administrator rights,
firewall, router or DNS changes. Nothing is shared unless you name installed packs here.
Windows may ask once whether Tonalli can use private networks; answer in that Windows prompt.

Examples:
  ./scripts/headless-task.ps1 -Action Install -Exe C:\Tonalli\tonalli-desktop.exe -SharePack my-book -StartNow
  ./scripts/headless-task.ps1 -Action Status
  ./scripts/headless-task.ps1 -Action Stop
  ./scripts/headless-task.ps1 -Action Remove
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Install', 'Status', 'Stop', 'Remove')][string]$Action,
    [string]$Exe,
    [string[]]$SharePack = @(),
    [string]$Address = 'auto',
    [string]$StatusFile = (Join-Path $env:LOCALAPPDATA 'Tonalli/headless-status.json'),
    [string]$TaskName = 'Tonalli headless sharing',
    [switch]$StartNow
)
$ErrorActionPreference = 'Stop'

function Read-Status {
    if (-not (Test-Path -LiteralPath $StatusFile)) { return $null }
    Get-Content -LiteralPath $StatusFile -Raw | ConvertFrom-Json
}

switch ($Action) {
    'Install' {
        if (-not $Exe -or -not (Test-Path -LiteralPath $Exe -PathType Leaf)) { throw 'Pass -Exe with the full path to tonalli-desktop.exe.' }
        if ($SharePack.Count -eq 0) { throw 'Pass at least one installed pack ID with -SharePack; nothing is shared by default.' }
        foreach ($id in $SharePack) { if ($id -notmatch '^[A-Za-z0-9._-]{1,64}$') { throw "Unsafe pack ID: $id" } }
        if ($Address -ne 'auto' -and ($Address -notmatch '^\d{1,3}(\.\d{1,3}){3}$' -or $Address -match '^(0\.|127\.)')) { throw 'Address must be auto or a private LAN IPv4 address.' }
        foreach ($path in @($Exe, $StatusFile)) { if ($path.Contains('"')) { throw 'Paths cannot contain a double quote.' } }
        $arguments = @('--headless', $Address, '--status-file', "`"$StatusFile`"") + ($SharePack | ForEach-Object { '--share-pack'; $_ })
        $action = New-ScheduledTaskAction -Execute $Exe -Argument ($arguments -join ' ')
        $trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
        $settings = New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -ExecutionTimeLimit ([TimeSpan]::Zero) -StartWhenAvailable -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
        $principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
        Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Force | Out-Null
        Write-Output "Registered '$TaskName' for this user. It starts at sign-in and shares only: $($SharePack -join ', ')."
        if ($StartNow) { Start-ScheduledTask -TaskName $TaskName; Write-Output 'Started. Run -Action Status in a few seconds for the address and pairing code.' }
    }
    'Status' {
        $task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
        Write-Output ($(if ($task) { "Task: $($task.State)" } else { 'Task: not installed' }))
        $status = Read-Status
        if ($status) { $status | ConvertTo-Json } else { Write-Output 'No status file yet.' }
    }
    'Stop' {
        New-Item -ItemType Directory -Force (Split-Path -Parent $StatusFile) | Out-Null
        Set-Content -LiteralPath "$StatusFile.stop" -Value 'stop' -Encoding ascii
        Write-Output 'Stop requested; sharing ends within a few seconds. Remove -Action Remove to stop it starting at sign-in.'
    }
    'Remove' {
        if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) { Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false }
        Write-Output "Removed '$TaskName'. Your library, notes and settings are untouched."
    }
}
