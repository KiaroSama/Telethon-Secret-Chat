<#
.SYNOPSIS
    Run the interop tier without a session string ever touching your shell history.

.DESCRIPTION
    The tier needs four variables (tests/interop/conftest.py) and three of them are
    already in the telegram-mcp `.env` next door. Typing a StringSession into a
    terminal to re-supply them puts a full login into your shell history, your
    scrollback and any terminal logging you have on - so this reads them from that
    file into THIS process only and runs pytest. Nothing is printed, and the values
    are never written anywhere.

    It also refuses to run while the telegram-mcp server is holding the same
    account. That is Principle I of the project constitution - one auth key, one
    connection - and Telegram enforces it by PERMANENTLY invalidating a key used
    from two places at once (AuthKeyDuplicatedError). Stopping the server is left
    to you rather than done here, because it is your running service.

    You are still the far end. The run prints what to do on the second account and
    waits for it with a deadline; that is the whole point of the tier - the
    evidence is an OFFICIAL client reading what this package wrote.

.PARAMETER Account
    Label of the account to run AS, matching TELEGRAM_SESSION_STRING_<LABEL> in the
    telegram-mcp .env. Case-insensitive; underscores as in the variable name.

.PARAMETER Peer
    The SECOND account - username or numeric id. This is the one you operate by
    hand during the run.

.PARAMETER MediaDir
    Optional. A directory of real video/audio samples. Without it those kinds are
    reported as not covered rather than faked.

.PARAMETER McpRoot
    Where to find the telegram-mcp `.env`. Defaults to the sibling checkout.

.EXAMPLE
    .\scripts\run_interop.ps1 -Account kgb_verifier -Peer @someone
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Account,
    [Parameter(Mandatory = $true)][string]$Peer,
    [string]$MediaDir,
    [string]$McpRoot = 'G:\Program Files\Portable\Scripts\Telegram-mcp'
)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $PSScriptRoot

function Fail([string]$message) {
    Write-Host "run_interop: $message" -ForegroundColor Red
    exit 1
}

$envFile = Join-Path $McpRoot '.env'
if (-not (Test-Path -LiteralPath $envFile)) {
    Fail "no .env at $envFile. Pass -McpRoot with the telegram-mcp checkout."
}

# Principle I. A session connected twice is a session Telegram destroys, so this
# is a refusal rather than a warning. The port is the server's own HTTP transport.
$listening = Get-NetTCPConnection -LocalPort 18765 -State Listen -ErrorAction SilentlyContinue
if ($listening) {
    Write-Host ""
    Write-Host "  The telegram-mcp server is running on 127.0.0.1:18765." -ForegroundColor Yellow
    Write-Host "  It holds this account's session, and Telegram PERMANENTLY invalidates" -ForegroundColor Yellow
    Write-Host "  an auth key used from two clients at once. Stop it, run this, start it" -ForegroundColor Yellow
    Write-Host "  again - the launcher is $McpRoot\start-mcp.ps1." -ForegroundColor Yellow
    Write-Host ""
    Fail "refusing to connect a second client to a live session."
}

# Parsed here rather than with python-dotenv so nothing leaves this process. Only
# the three names below are ever looked at; everything else in the file is ignored.
$wanted = @{
    ("TELEGRAM_SESSION_STRING_" + $Account.ToUpperInvariant()) = 'TSC_TEST_SESSION'
    'TELEGRAM_API_ID'                                          = 'TSC_TEST_API_ID'
    'TELEGRAM_API_HASH'                                        = 'TSC_TEST_API_HASH'
}
$found = @{}
foreach ($line in [IO.File]::ReadAllLines($envFile, [Text.UTF8Encoding]::new($false))) {
    $trimmed = $line.Trim()
    if ($trimmed.StartsWith('#') -or -not $trimmed.Contains('=')) { continue }
    $name = $trimmed.Substring(0, $trimmed.IndexOf('=')).Trim()
    if (-not $wanted.ContainsKey($name)) { continue }
    $value = $trimmed.Substring($trimmed.IndexOf('=') + 1).Trim().Trim('"').Trim("'")
    if ($value) { $found[$wanted[$name]] = $value }
}

# Names only in the report. A missing one is named; a present one never is.
$missing = @($wanted.Values | Where-Object { -not $found.ContainsKey($_) })
if ($missing.Count -gt 0) {
    Fail ("not in that .env: " + ($missing -join ', ') +
        " - check the account label, which must match TELEGRAM_SESSION_STRING_<LABEL>.")
}

foreach ($pair in $found.GetEnumerator()) {
    Set-Item -Path ("Env:" + $pair.Key) -Value $pair.Value
}
$env:TSC_TEST_PEER = $Peer
if ($MediaDir) {
    if (-not (Test-Path -LiteralPath $MediaDir -PathType Container)) {
        Fail "-MediaDir $MediaDir is not a directory."
    }
    $env:TSC_TEST_MEDIA_DIR = $MediaDir
}

Write-Host ""
Write-Host "  Running as '$Account' against peer '$Peer'." -ForegroundColor Cyan
Write-Host "  Have the SECOND account open in Telegram - the run prints each step" -ForegroundColor Cyan
Write-Host "  and waits up to 3 minutes for it. Accepting the chat is automatic:" -ForegroundColor Cyan
Write-Host "  the request reaches every device the peer has and the first to" -ForegroundColor Cyan
Write-Host "  complete the key exchange wins, which is normally the phone." -ForegroundColor Cyan
if (-not $MediaDir) {
    Write-Host "  No -MediaDir: video and audio will be reported as NOT covered." -ForegroundColor DarkYellow
}
Write-Host ""

Push-Location -LiteralPath $here
try {
    # `-s` is not optional: the instructions are printed while the run waits for
    # them, and capturing them buffers them past the deadline they exist to beat.
    & uv run --locked pytest tests/interop -q -s
    $code = $LASTEXITCODE
} finally {
    Pop-Location
    foreach ($name in @('TSC_TEST_SESSION', 'TSC_TEST_API_ID', 'TSC_TEST_API_HASH',
            'TSC_TEST_PEER', 'TSC_TEST_MEDIA_DIR')) {
        Remove-Item -Path ("Env:" + $name) -ErrorAction SilentlyContinue
    }
}

if ($code -eq 0) {
    Write-Host ""
    Write-Host "  Interop passed. SC-001 and SC-002 now have their evidence -" -ForegroundColor Green
    Write-Host "  record the date and this run in specs/.../tasks.md." -ForegroundColor Green
}
exit $code
