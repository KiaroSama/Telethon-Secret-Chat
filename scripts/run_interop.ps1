<#
.SYNOPSIS
    Run real-client interop using a private .env without exposing session values.
.DESCRIPTION
    Requires PowerShell 7 on Windows. The operator must stop every other user of
    the selected authorization. Port 18765 is a local safety check, not proof that
    no client on another machine uses that authorization. No service is stopped.
    All process environment values and the working directory are restored. A
    successful subset, skipped suite, or missing media never proves full interop.
    Each run also writes logs/run_interop_YYYY-MM-DD_HH-mm-ss_UTC.log under the
    repository root (UTF-8, one file per run, never overwritten, no secret values).
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidatePattern('^[A-Za-z0-9_]+$')][string]$Account,
    [Parameter(Mandatory = $true)][string]$Peer,
    [string]$MediaDir,
    [string]$Only,
    [string]$Nonce,
    [string]$McpRoot = 'G:\Program Files\Portable\Scripts\Telegram-mcp'
)

$ErrorActionPreference = 'Stop'
$started = [DateTime]::UtcNow
$utf8 = [Text.UTF8Encoding]::new($false)
$script:LogPath = $null
try {
    # Resolved from the script, not the caller's working directory.
    $logDir = Join-Path (Split-Path -Parent $PSScriptRoot) 'logs'
    [void][IO.Directory]::CreateDirectory($logDir)
    $stem = 'run_interop_' + $started.ToString('yyyy-MM-dd_HH-mm-ss') + '_UTC'
    for ($n = 1; -not $script:LogPath -and $n -le 100; $n++) {
        $candidate = Join-Path $logDir ($(if ($n -eq 1) { $stem } else { "${stem}_$n" }) + '.log')
        try {
            # CreateNew never replaces an earlier run's log, even one from the same second.
            [IO.File]::Open($candidate, [IO.FileMode]::CreateNew).Dispose()
            $script:LogPath = $candidate
        } catch [IO.IOException] { if (-not (Test-Path -LiteralPath $candidate)) { throw } }
    }
} catch {
    $script:LogPath = $null
}
if (-not $script:LogPath) { Write-Host 'WARNING: no run log file could be created; logging to the console only.' }
function Write-InteropLog {
    param([ValidateSet('INFO', 'WARNING', 'ERROR', 'DEBUG')][string]$Level, [string]$Message)
    if ($Level -eq 'DEBUG' -and $DebugPreference -eq 'SilentlyContinue') { return }
    Write-Host ("{0:yyyy-MM-ddTHH:mm:ss.fffK} [{1}] {2}" -f [DateTimeOffset]::Now, $Level, $Message)
    if ($script:LogPath) {
        try {
            $line = '[{0:yyyy-MM-dd HH:mm:ss} UTC] [{1}] [run_interop] {2}' -f [DateTime]::UtcNow, $Level, $Message
            [IO.File]::AppendAllText($script:LogPath, $line + "`n", $utf8)
        } catch {
            $script:LogPath = $null
            Write-Host 'WARNING: the run log file stopped accepting writes; logging to the console only.'
        }
    }
}
Write-InteropLog INFO ('Interop launcher started; run log: ' + $(if ($script:LogPath) { $script:LogPath } else { 'none' }))

$names = @('TSC_TEST_SESSION', 'TSC_TEST_API_ID', 'TSC_TEST_API_HASH',
    'TSC_TEST_PEER', 'TSC_TEST_MEDIA_DIR', 'TSC_TEST_NONCE')
$previous = @{}
foreach ($name in $names) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
$changed = $false
$pushed = $false
$record = $null
$code = 1
$failure = 'Preflight validation failed; no session was started.'
try {
    $envFile = Join-Path $McpRoot '.env'
    if (-not (Test-Path -LiteralPath $envFile -PathType Leaf)) { throw 'Missing .env' }
    if ($MediaDir -and -not (Test-Path -LiteralPath $MediaDir -PathType Container)) {
        $failure = 'MediaDir is not a directory; no session was started.'
        throw 'Invalid media directory'
    }
    $target = if ($Only) { $Only.Replace('\', '/') } else { 'tests/interop' }
    if ($target -notmatch '^tests/interop(?:/test_live_(?:roundtrip|media|rekey)\.py(?:::[A-Za-z_][A-Za-z0-9_]*)?)?$') {
        $failure = 'Only must select this repository''s live interop tier, not arbitrary pytest arguments.'
        throw 'Invalid selector'
    }
    $failure = 'Could not verify the local session-use guard; refusing to connect.'
    $listeners = @(Get-NetTCPConnection -State Listen -ErrorAction Stop)
    if (@($listeners | Where-Object { $_.LocalPort -eq 18765 }).Count) {
        $failure = 'The telegram-mcp listener is active; stop its session owner before interop.'
        throw 'Session already in use'
    }
    $failure = 'Could not read the required private account configuration.'
    $wanted = @{
        ('TELEGRAM_SESSION_STRING_' + $Account.ToUpperInvariant()) = 'TSC_TEST_SESSION'
        'TELEGRAM_API_ID' = 'TSC_TEST_API_ID'
        'TELEGRAM_API_HASH' = 'TSC_TEST_API_HASH'
    }
    $found = @{}
    foreach ($line in [IO.File]::ReadAllLines($envFile, [Text.UTF8Encoding]::new($false))) {
        $line = $line.Trim()
        if ($line.StartsWith('#') -or -not $line.Contains('=')) { continue }
        $index = $line.IndexOf('=')
        $name = $line.Substring(0, $index).Trim()
        if (-not $wanted.ContainsKey($name)) { continue }
        $value = $line.Substring($index + 1).Trim()
        if ($value.Length -ge 2 -and (($value.StartsWith('"') -and $value.EndsWith('"')) -or
                ($value.StartsWith("'") -and $value.EndsWith("'")))) {
            $value = $value.Substring(1, $value.Length - 2)
        }
        if ($value) { $found[$wanted[$name]] = $value }
    }
    if (@($wanted.Values | Where-Object { -not $found.ContainsKey($_) }).Count) {
        throw 'Missing required account fields'
    }
    $found['TSC_TEST_PEER'] = $Peer
    $found['TSC_TEST_MEDIA_DIR'] = if ($MediaDir) { (Resolve-Path -LiteralPath $MediaDir).Path } else { $null }
    $found['TSC_TEST_NONCE'] = if ($Nonce) { $Nonce } else { $null }
    $changed = $true
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $found[$name], 'Process') }
    Write-InteropLog INFO 'Starting the selected live cases. Follow the official-client instructions.'
    if (-not $MediaDir) { Write-InteropLog WARNING 'No real media directory: video/audio interoperability is NOT covered.' }
    Write-InteropLog DEBUG 'Process-only configuration prepared; values are intentionally not logged.'
    $failure = 'The test process could not complete; interoperability is not confirmed.'
    $record = [IO.Path]::GetTempFileName()
    Push-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)
    $pushed = $true
    Write-InteropLog INFO ('Running pytest on {0}; media directory given: {1}' -f $target, [bool]$MediaDir)
    & uv run --locked python -m pytest $target -q -s "--junitxml=$record"
    $code = $LASTEXITCODE
    Write-InteropLog INFO ('pytest exited with code {0}' -f $code)
    if ($code -ne 0) { throw 'Pytest did not succeed' }
    $failure = 'The test record is missing, incomplete, skipped, or failed; interoperability is not confirmed.'
    [xml]$xml = [IO.File]::ReadAllText($record)
    $cases = @($xml.SelectNodes('//testcase'))
    if (-not $cases.Count) { throw 'No executed cases' }
    foreach ($case in $cases) {
        if ($case.classname -notmatch '^tests\.interop\.test_live_(roundtrip|media|rekey)$' -or
            $case.SelectSingleNode('skipped|failure|error')) { throw 'Invalid test evidence' }
    }
    Write-InteropLog INFO ("The selected {0} live case(s) passed; record this exact selection and media coverage." -f $cases.Count)
    if ($Only) { Write-InteropLog WARNING 'This was a subset, not certification of the complete interop tier.' }
    $code = 0
} catch {
    Write-InteropLog ERROR $failure
    if ($code -eq 0) { $code = 1 }
} finally {
    if ($pushed) { Pop-Location }
    if ($changed) {
        foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
    }
    if ($record) { Remove-Item -LiteralPath $record -Force -ErrorAction SilentlyContinue }
    $level = if ($code -eq 0) { 'INFO' } else { 'ERROR' }
    Write-InteropLog $level ('Finished in {0:N1}s with exit code {1}' -f ([DateTime]::UtcNow - $started).TotalSeconds, $code)
}
exit $code
