#requires -Version 5.1

<#
.SYNOPSIS
Runs the offline and static safety checks for the minimal S3 raw SigV4 helper.

.PARAMETER Help
Prints usage without loading or executing the helper.
#>

[CmdletBinding()]
param([switch]$Help)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($Help) {
    'Usage: Test-S3Compat.RawSigV4.ps1 [-Help]'
    return
}

$helperPath = Join-Path $PSScriptRoot 'S3Compat.RawSigV4.ps1'
$clientsPath = Join-Path $PSScriptRoot 'S3Compat.Clients.ps1'
$entrypointPath = Join-Path $PSScriptRoot 'Invoke-PrismArkS3Compatibility.ps1'
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($helperPath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -ne 0) {
    throw "raw SigV4 helper has $($parseErrors.Count) parser error(s)"
}

$forbiddenCommands = @(
    'Add-Type', 'Invoke-Expression', 'Invoke-WebRequest', 'Invoke-RestMethod',
    'Start-Process', 'Write-Debug', 'Write-Host', 'Write-Information',
    'Write-Output', 'Write-Verbose', 'Write-Warning'
)
$commands = $ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.CommandAst]
}, $true)
foreach ($command in $commands) {
    $name = $command.GetCommandName()
    if ($null -ne $name -and $forbiddenCommands -contains $name) {
        throw "raw SigV4 helper contains forbidden command: $name"
    }
}

. $helperPath

if (-not (Test-S3RawSigV4Golden)) {
    throw 'raw SigV4 offline golden did not return success'
}

foreach ($invalidEndpoint in @(
    'ftp://127.0.0.1:9000',
    'http://user:password@127.0.0.1:9000',
    'http://127.0.0.1:9000/#fragment'
)) {
    $rejected = $false
    try {
        Resolve-S3RawEndpoint -Endpoint $invalidEndpoint | Out-Null
    }
    catch {
        $rejected = $true
    }
    if (-not $rejected) {
        throw "raw SigV4 endpoint safety check accepted: $invalidEndpoint"
    }
}

. $clientsPath

$duplicates = @($script:AwsOperations | Group-Object | Where-Object { $_.Count -ne 1 })
if ($duplicates.Count -ne 0) {
    throw "AWS operation list contains duplicates: $($duplicates.Name -join ', ')"
}
$clientSource = [IO.File]::ReadAllText($clientsPath)
foreach ($operation in $script:AwsOperations) {
    $pattern = '(?m)-Operation\s+[''\"]?' + [regex]::Escape($operation) + '[''\"]?(?:\s|$)'
    if ($clientSource -notmatch $pattern) {
        throw "AWS operation is listed but not implemented or explicitly classified: $operation"
    }
}
foreach ($operation in @(
    'Tagging.InvalidTag.Negative',
    'Tagging.DuplicateKey.Negative',
    'Tagging.TooMany.Negative',
    'Tagging.BadPercentEncoding.Negative'
)) {
    $skipPattern = '(?m)Add-SkipResult[^\r\n]+-Operation\s+[''\"]?' + [regex]::Escape($operation)
    if ($clientSource -match $skipPattern) {
        throw "raw Tagging negative is still classified as SKIP: $operation"
    }
}
$negativeSection = ($clientSource -split 'Invoke-CompatCase -Client aws -Operation RawSigV4\.SelfTest -Body \{', 2)[1]
$negativeSection = ($negativeSection -split 'Invoke-CompatCase -Client aws -Operation Versioning\.ExactRead -Body \{', 2)[0]
if ($negativeSection -match '(?i)list-object-versions|list-objects|--prefix') {
    throw 'raw Tagging negative section contains forbidden discovery or prefix-based cleanup'
}
if ($clientSource -notmatch '\$versionId\s*=\s*\[string\]\$Response\.VersionId' -or $clientSource -notmatch 'Add-AwsTrackedVersion[^\r\n]+-VersionId\s+\$versionId' -or $negativeSection -notmatch 'Register-AwsRawAcceptedVersion') {
    throw 'raw Tagging accepted-response cleanup is not tied to the exact response VersionId'
}
$entrypointSource = [IO.File]::ReadAllText($entrypointPath)
$rawSourceIndex = $entrypointSource.IndexOf("S3Compat.RawSigV4.ps1", [StringComparison]::Ordinal)
$clientSourceIndex = $entrypointSource.IndexOf("S3Compat.Clients.ps1", [StringComparison]::Ordinal)
if ($rawSourceIndex -lt 0 -or $clientSourceIndex -lt 0 -or $rawSourceIndex -gt $clientSourceIndex) {
    throw 'compatibility entrypoint does not load the raw helper before the client matrix'
}

'PASS: raw SigV4 goldens, endpoint/AST guards, exact-version cleanup, and operation synchronization'
