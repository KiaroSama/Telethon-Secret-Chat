#Requires -Version 7.0
<#
.SYNOPSIS
    Refuse the retired Python live-interoperability launcher.
.DESCRIPTION
    The Rust rewrite does not ship a runnable live Telegram interoperability
    harness. This compatibility entry point exits with code 2 before reading
    account files, changing environment variables, or starting a session.
    Archived captures and offline Cargo tests do not certify a live exchange.
#>
[CmdletBinding()]
param(
    [string]$Account,
    [string]$Peer,
    [string]$MediaDir,
    [string]$Only,
    [string]$Nonce,
    [string]$PeerAccount,
    [string]$McpRoot
)

function Write-InteropLog {
    param(
        [ValidateSet('INFO', 'WARNING', 'ERROR', 'DEBUG')][string]$Level,
        [string]$Message
    )
    if ($Level -eq 'DEBUG' -and $DebugPreference -eq 'SilentlyContinue') { return }
    $stamp = [DateTimeOffset]::UtcNow.ToString('o', [Globalization.CultureInfo]::InvariantCulture)
    Write-Host "$stamp [$Level] $Message"
}

Write-InteropLog INFO 'The legacy Python interoperability launcher has been retired.'
Write-InteropLog WARNING 'The current Rust project has offline tests and archived evidence only.'
Write-InteropLog DEBUG 'No account configuration was read and no Telegram session was started.'
Write-InteropLog ERROR 'A native live-interoperability harness is not implemented; this command cannot run live tests.'
exit 2
