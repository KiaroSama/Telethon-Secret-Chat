<# Synthetic contract tests: no Telegram session, network connection, or external pytest. #>
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
function Write-TestLog {
    param([ValidateSet('INFO', 'WARNING', 'ERROR', 'DEBUG')][string]$Level, [string]$Message)
    if ($Level -eq 'DEBUG' -and $DebugPreference -eq 'SilentlyContinue') { return }
    Write-Host ("{0:yyyy-MM-ddTHH:mm:ss.fffK} [{1}] {2}" -f [DateTimeOffset]::Now, $Level, $Message)
}
function Assert-Contract([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function global:Get-NetTCPConnection {
    param($State, $ErrorAction)
    if ($global:InteropMode -eq 'guard-error') { throw 'Synthetic guard failure' }
    if ($global:InteropMode -eq 'listener') { [pscustomobject]@{LocalPort = 18765} }
}
function global:uv {
    $global:InteropCalls++
    if ($global:InteropMode -eq 'invoke-error') { throw 'Synthetic process failure' }
    $arg = @($args | Where-Object { $_ -like '--junitxml=*' })[0]
    $path = $arg.Substring('--junitxml='.Length)
    $status = switch ($global:InteropMode) {
        'skip' { '<skipped/>' }
        'failed' { '<failure/>' }
        default { '' }
    }
    $case = if ($global:InteropMode -eq 'empty') { '' } else {
        '<testcase classname="tests.interop.test_live_roundtrip" name="test_both_ends_agree_on_the_key_fingerprint">' + $status + '</testcase>'
    }
    [IO.File]::WriteAllText($path, '<testsuites><testsuite>' + $case + '</testsuite></testsuites>')
    $global:LASTEXITCODE = if ($global:InteropMode -eq 'failed') { 1 } else { 0 }
}
$root = Join-Path ([IO.Path]::GetTempPath()) ('interop-launcher-' + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $root | Out-Null
$launcher = Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) 'scripts/run_interop.ps1'
$logDir = Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) 'logs'
$logLine = '^\[\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} UTC\] \[(DEBUG|INFO|WARNING|ERROR)\] \[run_interop\] \S'
$created = [Collections.Generic.List[string]]::new()
function Get-RunLogs { @(Get-ChildItem -LiteralPath $logDir -Filter 'run_interop_*_UTC*.log' -File -ErrorAction SilentlyContinue | ForEach-Object FullName) }
$existing = Get-RunLogs
$names = @('TSC_TEST_SESSION', 'TSC_TEST_API_ID', 'TSC_TEST_API_HASH', 'TSC_TEST_PEER', 'TSC_TEST_MEDIA_DIR', 'TSC_TEST_NONCE')
$before = @{}
foreach ($name in $names) { $before[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    [IO.File]::WriteAllText((Join-Path $root '.env'), "TELEGRAM_SESSION_STRING_TEST='SYNTHETIC_NEVER_LOGIN'`nTELEGRAM_API_ID=1`nTELEGRAM_API_HASH='SYNTHETIC_HASH'`n")
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, 'PREEXISTING_' + $name, 'Process') }
    foreach ($mode in @('guard-error', 'listener', 'bad-media', 'bad-selector', 'invoke-error', 'empty', 'skip', 'failed', 'success')) {
        $global:InteropMode = $mode
        $global:InteropCalls = 0
        $location = (Get-Location).Path
        $arguments = @{Account = 'test'; Peer = '@synthetic'; McpRoot = $root}
        if ($mode -eq 'bad-media') { $arguments.MediaDir = Join-Path $root 'absent' }
        if ($mode -eq 'bad-selector') { $arguments.Only = 'tests/unit' }
        if ($mode -eq 'success') { $arguments.Only = 'tests/interop/test_live_roundtrip.py::test_both_ends_agree_on_the_key_fingerprint' }
        $output = (& $launcher @arguments 6>&1 | Out-String)
        $result = $LASTEXITCODE
        Assert-Contract (($result -eq 0) -eq ($mode -eq 'success')) ('Unexpected exit status for ' + $mode)
        Assert-Contract ((Get-Location).Path -eq $location) 'Working directory was not restored'
        foreach ($name in $names) {
            Assert-Contract ([Environment]::GetEnvironmentVariable($name, 'Process') -eq ('PREEXISTING_' + $name)) 'A process environment value was not restored'
        }
        Assert-Contract ($output -notmatch 'SYNTHETIC_NEVER_LOGIN|SYNTHETIC_HASH') 'Credential appeared in output'
        Assert-Contract ($output -notmatch 'SC-001 and SC-002 now have') 'Subset falsely claimed complete evidence'
        # One new log file per run, never an overwrite: runs in the same second get a suffix.
        $new = @(Get-RunLogs | Where-Object { $_ -notin $existing -and $_ -notin $created })
        Assert-Contract ($new.Count -eq 1) ('Expected exactly one new run log for ' + $mode)
        $created.Add($new[0])
        $lines = [IO.File]::ReadAllLines($new[0], [Text.UTF8Encoding]::new($false, $true))
        Assert-Contract ($lines.Count -ge 2) 'The run log records neither start nor outcome'
        Assert-Contract (@($lines | Where-Object { $_ -notmatch $logLine }).Count -eq 0) 'A run log line is not [UTC time] [LEVEL] [run_interop] message'
        Assert-Contract (($lines -join "`n") -notmatch 'SYNTHETIC_NEVER_LOGIN|SYNTHETIC_HASH|@synthetic') 'A private value reached the run log'
        Assert-Contract ($lines[-1] -match ('exit code ' + $result + '$')) 'The run log does not end with the exit code'
        if ($mode -in @('guard-error', 'listener', 'bad-media', 'bad-selector')) {
            Assert-Contract ($global:InteropCalls -eq 0) 'Preflight refusal still launched a process'
        }
        Write-TestLog INFO ('Launcher contract passed: ' + $mode)
    }
    Write-TestLog WARNING 'Only synthetic launcher behavior was tested; no live Telegram evidence was collected.'
} catch {
    Write-TestLog ERROR ('Launcher contract failed: ' + $_.Exception.Message)
    exit 1
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $before[$name], 'Process') }
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
    foreach ($path in $created) { Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue }
    Remove-Item Function:global:uv, Function:global:Get-NetTCPConnection -ErrorAction SilentlyContinue
    Remove-Variable InteropMode, InteropCalls -Scope Global -ErrorAction SilentlyContinue
}
