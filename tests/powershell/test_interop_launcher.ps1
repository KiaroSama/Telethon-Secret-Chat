#Requires -Version 7.0
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
# Stands in for `uv run ... pytest`: writes the junit record a real run would, with the
# classname of the module the launcher selected, so the evidence check is exercised.
function global:uv {
    $global:InteropCalls++
    if ($global:InteropMode -eq 'invoke-error') { throw 'Synthetic process failure' }
    $arg = @($args | Where-Object { $_ -like '--junitxml=*' })[0]
    $path = $arg.Substring('--junitxml='.Length)
    $target = @($args | Where-Object { $_ -like 'tests/interop*' })[0]
    $module = if ($target -match 'test_live_([a-z_]+)\.py') { 'test_live_' + $Matches[1] } else { 'test_live_roundtrip' }
    $classname = if ($global:InteropMode -eq 'foreign-evidence') { 'tests.unit.example' } else { "tests.interop.$module" }
    # The launcher must hand the test process what it was given; a mismatch is a failure.
    $wrongEnv = ($global:InteropMode -eq 'nonce' -and $env:TSC_TEST_NONCE -ne $global:InteropNonce) -or
        ($global:InteropMode -eq 'media-ok' -and $env:TSC_TEST_MEDIA_DIR -ne $global:InteropMedia)
    $status = if ($global:InteropMode -eq 'skip') { '<skipped/>' }
        elseif ($global:InteropMode -eq 'failed' -or $wrongEnv) { '<failure/>' } else { '' }
    $case = if ($global:InteropMode -eq 'empty') { '' } else {
        "<testcase classname=`"$classname`" name=`"test_synthetic`">" + $status + '</testcase>'
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
foreach ($name in $names + 'TSC_MCP_ROOT') { $before[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
$envFile = Join-Path $root '.env'
$fullEnv = "TELEGRAM_SESSION_STRING_TEST='SYNTHETIC_NEVER_LOGIN'`nTELEGRAM_API_ID=1`nTELEGRAM_API_HASH='SYNTHETIC_HASH'`n"
$global:InteropNonce = 'SYNTHETIC_NONCE_7f3a'
$mediaDir = Join-Path $root 'media'
New-Item -ItemType Directory -Path $mediaDir | Out-Null
$global:InteropMedia = (Resolve-Path -LiteralPath $mediaDir).Path
$preflight = 'Preflight validation failed; no session was started.'
$process = 'The test process could not complete; interoperability is not confirmed.'
$evidence = 'The test record is missing, incomplete, skipped, or failed; interoperability is not confirmed.'
# Every refusal and the exact ERROR text its run log must carry.
$refusals = [ordered]@{
    'no-env' = 'No .env was found under the telegram-mcp root; pass -McpRoot or set TSC_MCP_ROOT.'
    'missing-field' = 'Could not read the required private account configuration.'
    'guard-error' = 'Could not verify the local session-use guard; refusing to connect.'
    'listener' = 'The telegram-mcp listener is active; stop its session owner before interop.'
    'bad-media' = 'MediaDir is not a directory; no session was started.'
    'bad-selector' = 'Only must select this repository''s live interop tier, not arbitrary pytest arguments.'
    'invoke-error' = $process
    'failed' = $process
    'empty' = $evidence
    'skip' = $evidence
    'foreign-evidence' = $evidence
}
$preflightModes = @('no-env', 'missing-field', 'guard-error', 'listener', 'bad-media', 'bad-selector')
$selectors = @{
    'success-roundtrip' = 'tests/interop/test_live_roundtrip.py::test_both_ends_agree_on_the_key_fingerprint'
    'success-media' = 'tests/interop/test_live_media.py'
    'success-rekey' = 'tests/interop/test_live_rekey.py'
}
$successes = @('success-roundtrip', 'success-media', 'success-rekey', 'success-all', 'media-ok', 'nonce', 'env-root')
try {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, 'PREEXISTING_' + $name, 'Process') }
    [Environment]::SetEnvironmentVariable('TSC_MCP_ROOT', $null, 'Process')
    foreach ($mode in @($refusals.Keys) + $successes) {
        $global:InteropMode = $mode
        $global:InteropCalls = 0
        [IO.File]::WriteAllText($envFile, $fullEnv)
        if ($mode -eq 'no-env') { Remove-Item -LiteralPath $envFile }
        if ($mode -eq 'missing-field') {
            [IO.File]::WriteAllText($envFile, "TELEGRAM_SESSION_STRING_TEST='SYNTHETIC_NEVER_LOGIN'`nTELEGRAM_API_ID=1`n")
        }
        $location = (Get-Location).Path
        $arguments = @{Account = 'test'; Peer = '@synthetic'; McpRoot = $root}
        if ($mode -eq 'env-root') {
            # The default must come from TSC_MCP_ROOT, not from a path baked into the script.
            $arguments.Remove('McpRoot')
            [Environment]::SetEnvironmentVariable('TSC_MCP_ROOT', $root, 'Process')
        }
        if ($mode -eq 'bad-media') { $arguments.MediaDir = Join-Path $root 'absent' }
        if ($mode -eq 'media-ok') { $arguments.MediaDir = $mediaDir }
        if ($mode -eq 'nonce') { $arguments.Nonce = $global:InteropNonce }
        if ($mode -eq 'bad-selector') { $arguments.Only = 'tests/unit' }
        if ($selectors.ContainsKey($mode)) { $arguments.Only = $selectors[$mode] }
        $output = (& $launcher @arguments 6>&1 | Out-String)
        $result = $LASTEXITCODE
        [Environment]::SetEnvironmentVariable('TSC_MCP_ROOT', $null, 'Process')
        $succeeds = $mode -in $successes
        Assert-Contract (($result -eq 0) -eq $succeeds) ('Unexpected exit status for ' + $mode)
        Assert-Contract ((Get-Location).Path -eq $location) 'Working directory was not restored'
        foreach ($name in $names) {
            Assert-Contract ([Environment]::GetEnvironmentVariable($name, 'Process') -eq ('PREEXISTING_' + $name)) 'A process environment value was not restored'
        }
        Assert-Contract ($output -notmatch "SYNTHETIC_NEVER_LOGIN|SYNTHETIC_HASH|$($global:InteropNonce)") 'Credential or nonce appeared in output'
        # One new log file per run, never an overwrite: runs in the same second get a suffix.
        $new = @(Get-RunLogs | Where-Object { $_ -notin $existing -and $_ -notin $created })
        Assert-Contract ($new.Count -eq 1) ('Expected exactly one new run log for ' + $mode)
        $created.Add($new[0])
        $lines = [IO.File]::ReadAllLines($new[0], [Text.UTF8Encoding]::new($false, $true))
        Assert-Contract ($lines.Count -ge 2) 'The run log records neither start nor outcome'
        Assert-Contract (@($lines | Where-Object { $_ -notmatch $logLine }).Count -eq 0) 'A run log line is not [UTC time] [LEVEL] [run_interop] message'
        Assert-Contract (($lines -join "`n") -notmatch "SYNTHETIC_NEVER_LOGIN|SYNTHETIC_HASH|@synthetic|$($global:InteropNonce)") 'A private value reached the run log'
        Assert-Contract ($lines[-1] -match ('exit code ' + $result + '$')) 'The run log does not end with the exit code'
        if (-not $succeeds) {
            $expected = '[ERROR] [run_interop] ' + $refusals[$mode]
            Assert-Contract (@($lines | Where-Object { $_.EndsWith($expected) }).Count -eq 1) ('The run log does not name the reason for ' + $mode)
        }
        if ($mode -in $preflightModes) {
            Assert-Contract ($global:InteropCalls -eq 0) 'Preflight refusal still launched a process'
        }
        if ($succeeds) {
            $subset = [bool]($lines -match 'This was a subset')
            Assert-Contract ($subset -eq $arguments.ContainsKey('Only')) ('Subset warning wrong for ' + $mode)
            $noMedia = [bool]($lines -match 'video/audio interoperability is NOT covered')
            Assert-Contract ($noMedia -eq ($mode -ne 'media-ok')) ('Media coverage warning wrong for ' + $mode)
        }
        Write-TestLog INFO ('Launcher contract passed: ' + $mode)
    }
    Write-TestLog WARNING 'Only synthetic launcher behavior was tested; no live Telegram evidence was collected.'
} catch {
    Write-TestLog ERROR ('Launcher contract failed: ' + $_.Exception.Message)
    exit 1
} finally {
    foreach ($name in $names + 'TSC_MCP_ROOT') { [Environment]::SetEnvironmentVariable($name, $before[$name], 'Process') }
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
    foreach ($path in $created) { Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue }
    Remove-Item Function:global:uv, Function:global:Get-NetTCPConnection -ErrorAction SilentlyContinue
    Remove-Variable InteropMode, InteropCalls, InteropNonce, InteropMedia -Scope Global -ErrorAction SilentlyContinue
}
