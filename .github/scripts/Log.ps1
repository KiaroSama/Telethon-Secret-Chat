# Shared CI logging: fixed diagnostics only; never pass credentials to this function.
function Write-CiLog {
    param(
        [ValidateSet('INFO', 'WARNING', 'ERROR', 'DEBUG')][string]$Level,
        [string]$Message
    )
    if ($Level -eq 'DEBUG' -and $env:CI_DEBUG -ne '1') { return }
    $stamp = [DateTimeOffset]::UtcNow.ToString('o', [Globalization.CultureInfo]::InvariantCulture)
    Write-Host "$stamp [$Level] $Message"
}
