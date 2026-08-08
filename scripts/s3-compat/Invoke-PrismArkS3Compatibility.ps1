#requires -Version 5.1

<#
.SYNOPSIS
Runs the repeatable PrismArk S3 client compatibility matrix.

.DESCRIPTION
Detects AWS CLI, mcli/mc, and rclone, then exercises every client that is
available. DockerLocal and DockerSilo create isolated, disposable Docker
resources. Endpoint tests an already-running PrismArk S3 endpoint.

.PARAMETER Target
DockerLocal starts PostgreSQL and PrismArk with ephemeral local storage.
DockerSilo additionally starts pgsty/silo as PrismArk's storage backend.
Endpoint tests an existing endpoint and reads credentials from
PRISMARK_S3_ACCESS_KEY_ID and PRISMARK_S3_SECRET_ACCESS_KEY.

.PARAMETER Endpoint
S3 endpoint for Target=Endpoint, for example http://127.0.0.1:9000.

.PARAMETER SkipBuild
Uses PrismArkImage without building it from the current checkout.

.PARAMETER Help
Prints concise usage without checking Docker or clients.

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\s3-compat\Invoke-PrismArkS3Compatibility.ps1 -Target DockerLocal

.EXAMPLE
$env:PRISMARK_S3_ACCESS_KEY_ID = '<access-key-id>'
$env:PRISMARK_S3_SECRET_ACCESS_KEY = '<secret-access-key>'
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\s3-compat\Invoke-PrismArkS3Compatibility.ps1 -Target Endpoint -Endpoint http://127.0.0.1:9000
#>

[CmdletBinding()]
param(
    [ValidateSet('DockerLocal', 'DockerSilo', 'Endpoint')]
    [string]$Target = 'DockerLocal',

    [string]$Endpoint,

    [ValidatePattern('^[a-z0-9][a-z0-9-]{0,62}$')]
    [string]$Region = 'us-east-1',

    [string]$PrismArkImage = 'prismark-s3-compat:local',

    [string]$SiloImage = 'docker.io/pgsty/silo:latest',

    [string]$PostgresImage = 'postgres:17-bookworm',

    [switch]$SkipBuild,

    [string]$OutputDirectory,

    [ValidateRange(30, 900)]
    [int]$TimeoutSeconds = 180,

    [switch]$Help
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $PSScriptRoot 'results'
}

if ($Help) {
    @'
PrismArk S3 compatibility matrix

Usage:
  Invoke-PrismArkS3Compatibility.ps1 [-Target DockerLocal|DockerSilo|Endpoint]
      [-Endpoint URL] [-Region REGION] [-PrismArkImage IMAGE] [-SkipBuild]
      [-OutputDirectory PATH] [-TimeoutSeconds 30..900]

Endpoint mode credentials (environment only):
  PRISMARK_S3_ACCESS_KEY_ID
  PRISMARK_S3_SECRET_ACCESS_KEY

The script never installs clients. Missing aws, mcli/mc, or rclone clients are
reported as SKIP. Reports are JSON and Markdown. Any FAIL returns exit code 1;
PASS, SKIP, and XFAIL return 0 when no FAIL exists.
'@ | Write-Output
    return
}

$script:KnownSecrets = New-Object System.Collections.Generic.List[string]
$script:Results = New-Object System.Collections.Generic.List[object]
$script:EnvironmentBackup = @{}
$script:RunId = ([Guid]::NewGuid().ToString('N')).Substring(0, 12)
$script:StartedAt = [DateTimeOffset]::UtcNow
$script:TempRoot = Join-Path ([IO.Path]::GetTempPath()) ("prismark-s3-compat-{0}" -f $script:RunId)
$script:RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$script:DockerResources = [ordered]@{
    Network = $null
    Postgres = $null
    Silo = $null
    PrismArk = $null
}

function Register-Secret {
    param([AllowEmptyString()][string]$Value)
    if (-not [string]::IsNullOrEmpty($Value) -and -not $script:KnownSecrets.Contains($Value)) {
        $script:KnownSecrets.Add($Value)
    }
}

function Protect-Text {
    param([AllowNull()][object]$Value)
    if ($null -eq $Value) {
        return ''
    }
    $protected = [string]$Value
    foreach ($secret in $script:KnownSecrets) {
        if ([string]::IsNullOrEmpty($secret)) {
            continue
        }
        $protected = $protected.Replace($secret, '<redacted>')
        $escaped = [Uri]::EscapeDataString($secret)
        if ($escaped -ne $secret) {
            $protected = $protected.Replace($escaped, '<redacted>')
        }
    }
    return $protected
}

function Write-CompatInfo {
    param([string]$Message)
    Write-Host (Protect-Text ("[s3-compat] {0}" -f $Message))
}

function Set-ScopedEnvironmentVariable {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [AllowNull()][string]$Value
    )
    if (-not $script:EnvironmentBackup.ContainsKey($Name)) {
        $script:EnvironmentBackup[$Name] = [Environment]::GetEnvironmentVariable($Name, 'Process')
    }
    [Environment]::SetEnvironmentVariable($Name, $Value, 'Process')
}

function Restore-ScopedEnvironmentVariables {
    foreach ($name in $script:EnvironmentBackup.Keys) {
        [Environment]::SetEnvironmentVariable($name, $script:EnvironmentBackup[$name], 'Process')
    }
    $script:EnvironmentBackup.Clear()
}

function New-RandomSecret {
    param([ValidateRange(16, 128)][int]$Bytes = 32)
    $buffer = New-Object byte[] $Bytes
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $rng.GetBytes($buffer)
    }
    finally {
        $rng.Dispose()
    }
    return [Convert]::ToBase64String($buffer).TrimEnd('=').Replace('+', '-').Replace('/', '_')
}

function New-Base64Key {
    $buffer = New-Object byte[] 32
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $rng.GetBytes($buffer)
    }
    finally {
        $rng.Dispose()
    }
    return [Convert]::ToBase64String($buffer)
}

function Invoke-NativeCommand {
    param(
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [switch]$AllowFailure,
        [int]$MaxDiagnosticCharacters = 3000
    )
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $lines = @(& $Command @Arguments 2>&1 | ForEach-Object { $_.ToString() })
    $exitCode = $LASTEXITCODE
    $watch.Stop()
    $output = Protect-Text (($lines -join [Environment]::NewLine).Trim())
    if ($output.Length -gt $MaxDiagnosticCharacters) {
        $output = $output.Substring($output.Length - $MaxDiagnosticCharacters)
    }
    $result = [pscustomobject]@{
        ExitCode = $exitCode
        Output = $output
        DurationMs = $watch.ElapsedMilliseconds
    }
    if (-not $AllowFailure -and $exitCode -ne 0) {
        $diagnostic = if ([string]::IsNullOrWhiteSpace($output)) { 'no diagnostic output' } else { $output }
        throw "native command failed with exit code ${exitCode}: $diagnostic"
    }
    return $result
}

function Get-CompatibilityBaseline {
    $git = Get-Command git -ErrorAction SilentlyContinue
    if ($null -eq $git) {
        return 'current-worktree'
    }
    $revision = Invoke-NativeCommand -Command $git.Source -Arguments @('-C', $script:RepositoryRoot, 'rev-parse', '--short=12', 'HEAD') -AllowFailure
    if ($revision.ExitCode -ne 0 -or [string]::IsNullOrWhiteSpace($revision.Output)) {
        return 'current-worktree'
    }
    $status = Invoke-NativeCommand -Command $git.Source -Arguments @('-C', $script:RepositoryRoot, 'status', '--porcelain') -AllowFailure
    $suffix = if ($status.ExitCode -eq 0 -and -not [string]::IsNullOrWhiteSpace($status.Output)) { '+dirty' } else { '' }
    return "$($revision.Output.Trim())$suffix"
}

function Add-CompatResult {
    param(
        [Parameter(Mandatory = $true)][string]$Client,
        [Parameter(Mandatory = $true)][string]$Operation,
        [ValidateSet('PASS', 'SKIP', 'XFAIL', 'FAIL')][string]$Status,
        [long]$DurationMs = 0,
        [string]$Message = ''
    )
    $safeMessage = Protect-Text $Message
    $script:Results.Add([pscustomobject][ordered]@{
        client = $Client
        operation = $Operation
        status = $Status
        duration_ms = $DurationMs
        message = $safeMessage
    })
    Write-CompatInfo ("{0,-7} {1}/{2} - {3}" -f $Status, $Client, $Operation, $safeMessage)
}

function Invoke-CompatCase {
    param(
        [Parameter(Mandatory = $true)][string]$Client,
        [Parameter(Mandatory = $true)][string]$Operation,
        [Parameter(Mandatory = $true)][scriptblock]$Body,
        [string]$SuccessMessage = 'operation completed'
    )
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        $message = & $Body
        $watch.Stop()
        if ($null -eq $message -or [string]::IsNullOrWhiteSpace([string]$message)) {
            $message = $SuccessMessage
        }
        Add-CompatResult -Client $Client -Operation $Operation -Status PASS -DurationMs $watch.ElapsedMilliseconds -Message ([string]$message)
        return $true
    }
    catch {
        $watch.Stop()
        Add-CompatResult -Client $Client -Operation $Operation -Status FAIL -DurationMs $watch.ElapsedMilliseconds -Message $_.Exception.Message
        return $false
    }
}

function Invoke-ExpectedFailureCase {
    param(
        [Parameter(Mandatory = $true)][string]$Client,
        [Parameter(Mandatory = $true)][string]$Operation,
        [Parameter(Mandatory = $true)][scriptblock]$Body,
        [Parameter(Mandatory = $true)][string]$Reason,
        [string]$ExpectedPattern = '(?i)(NotImplemented|Not Implemented|Unsupported|501)'
    )
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        & $Body | Out-Null
        $watch.Stop()
        Add-CompatResult -Client $Client -Operation $Operation -Status PASS -DurationMs $watch.ElapsedMilliseconds -Message 'previously unsupported operation now succeeds'
        return $true
    }
    catch {
        $watch.Stop()
        $message = Protect-Text $_.Exception.Message
        if ($message -match $ExpectedPattern) {
            Add-CompatResult -Client $Client -Operation $Operation -Status XFAIL -DurationMs $watch.ElapsedMilliseconds -Message $Reason
            return $true
        }
        Add-CompatResult -Client $Client -Operation $Operation -Status FAIL -DurationMs $watch.ElapsedMilliseconds -Message ("unexpected failure while checking an expected gap: {0}" -f $message)
        return $false
    }
}

function Add-SkipResult {
    param([string]$Client, [string]$Operation, [string]$Reason)
    Add-CompatResult -Client $Client -Operation $Operation -Status SKIP -Message $Reason
}

function ConvertFrom-CompatJson {
    param([string]$Json, [string]$Context)
    try {
        return $Json | ConvertFrom-Json
    }
    catch {
        throw "${Context} returned invalid JSON: $(Protect-Text $_.Exception.Message)"
    }
}

function Assert-Condition {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) {
        throw $Message
    }
}

function Get-FileSha256 {
    param([string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-NativeClient {
    param([string[]]$Names)
    foreach ($name in $Names) {
        $command = Get-Command $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($null -ne $command) {
            return [pscustomobject]@{ Name = $name; Path = $command.Source }
        }
    }
    return $null
}

function Wait-Until {
    param(
        [Parameter(Mandatory = $true)][scriptblock]$Probe,
        [Parameter(Mandatory = $true)][string]$Description,
        [int]$Timeout = $TimeoutSeconds
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($Timeout)
    $lastError = ''
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            if (& $Probe) {
                return
            }
        }
        catch {
            $lastError = Protect-Text $_.Exception.Message
        }
        Start-Sleep -Milliseconds 750
    }
    if ([string]::IsNullOrWhiteSpace($lastError)) {
        throw "timed out waiting for $Description"
    }
    throw "timed out waiting for ${Description}: $lastError"
}

function Get-DockerPublishedPort {
    param([string]$Container, [int]$ContainerPort)
    $mapping = Invoke-NativeCommand -Command docker -Arguments @('port', $Container, "${ContainerPort}/tcp")
    $line = ($mapping.Output -split "`r?`n" | Select-Object -First 1).Trim()
    if ($line -notmatch ':(\d+)$') {
        throw "Docker returned an invalid port mapping for ${ContainerPort}/tcp: $line"
    }
    return [int]$Matches[1]
}

function Get-ContainerDiagnostic {
    param([string]$Container)
    if ([string]::IsNullOrWhiteSpace($Container)) {
        return ''
    }
    $logs = Invoke-NativeCommand -Command docker -Arguments @('logs', '--tail', '120', $Container) -AllowFailure
    return Protect-Text $logs.Output
}

function Start-IsolatedStack {
    param([ValidateSet('DockerLocal', 'DockerSilo')][string]$Mode)

    Invoke-NativeCommand -Command docker -Arguments @('version', '--format', '{{.Server.Version}}') | Out-Null
    $repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
    if (-not $SkipBuild) {
        Write-CompatInfo "building PrismArk image $PrismArkImage from baseline checkout"
        Invoke-NativeCommand -Command docker -Arguments @('build', '--tag', $PrismArkImage, '--file', (Join-Path $repositoryRoot 'Dockerfile'), $repositoryRoot) -MaxDiagnosticCharacters 8000 | Out-Null
    }
    else {
        Invoke-NativeCommand -Command docker -Arguments @('image', 'inspect', $PrismArkImage) | Out-Null
    }

    $network = "prismark-compat-net-$($script:RunId)"
    $postgres = "prismark-compat-pg-$($script:RunId)"
    $silo = "prismark-compat-silo-$($script:RunId)"
    $prismark = "prismark-compat-app-$($script:RunId)"
    $script:DockerResources.Network = $network
    $script:DockerResources.Postgres = $postgres
    $script:DockerResources.PrismArk = $prismark

    Invoke-NativeCommand -Command docker -Arguments @('network', 'create', '--label', "prismark.s3-compat.run=$($script:RunId)", $network) | Out-Null

    $postgresPassword = New-RandomSecret 36
    Register-Secret $postgresPassword
    Set-ScopedEnvironmentVariable POSTGRES_DB 'prismark'
    Set-ScopedEnvironmentVariable POSTGRES_USER 'prismark'
    Set-ScopedEnvironmentVariable POSTGRES_PASSWORD $postgresPassword
    Invoke-NativeCommand -Command docker -Arguments @(
        'run', '--detach', '--rm', '--name', $postgres,
        '--network', $network, '--network-alias', 'postgres',
        '--label', "prismark.s3-compat.run=$($script:RunId)",
        '--env', 'POSTGRES_DB', '--env', 'POSTGRES_USER', '--env', 'POSTGRES_PASSWORD',
        $PostgresImage
    ) | Out-Null
    Wait-Until -Description 'PostgreSQL readiness' -Probe {
        $probe = Invoke-NativeCommand -Command docker -Arguments @('exec', $postgres, 'pg_isready', '-U', 'prismark', '-d', 'prismark') -AllowFailure
        return $probe.ExitCode -eq 0
    }

    $siloAccessKey = $null
    $siloSecretKey = $null
    $siloBucket = $null
    if ($Mode -eq 'DockerSilo') {
        $script:DockerResources.Silo = $silo
        $siloAccessKey = "silo$($script:RunId)"
        $siloSecretKey = New-RandomSecret 36
        $siloBucket = "prismark-objects-$($script:RunId)"
        Register-Secret $siloSecretKey
        Set-ScopedEnvironmentVariable MINIO_ROOT_USER $siloAccessKey
        Set-ScopedEnvironmentVariable MINIO_ROOT_PASSWORD $siloSecretKey
        Invoke-NativeCommand -Command docker -Arguments @(
            'run', '--detach', '--rm', '--name', $silo,
            '--network', $network, '--network-alias', 'silo',
            '--label', "prismark.s3-compat.run=$($script:RunId)",
            '--env', 'MINIO_ROOT_USER', '--env', 'MINIO_ROOT_PASSWORD',
            $SiloImage, 'server', '/data', '--console-address', ':9001'
        ) | Out-Null
        Wait-Until -Description 'Silo readiness' -Probe {
            $alias = Invoke-NativeCommand -Command docker -Arguments @('exec', $silo, 'mcli', 'alias', 'set', 'compat', 'http://127.0.0.1:9000', $siloAccessKey, $siloSecretKey, '--api', 'S3v4') -AllowFailure
            return $alias.ExitCode -eq 0
        }
        Invoke-NativeCommand -Command docker -Arguments @('exec', $silo, 'mcli', 'mb', "compat/$siloBucket") | Out-Null
    }

    $masterKey = New-Base64Key
    $mediaKey = New-Base64Key
    Register-Secret $masterKey
    Register-Secret $mediaKey
    $databaseUrl = "postgres://prismark:${postgresPassword}@postgres:5432/prismark"
    Register-Secret $databaseUrl
    Set-ScopedEnvironmentVariable MEDIAHUB_BIND_ADDR '0.0.0.0:3000'
    Set-ScopedEnvironmentVariable MEDIAHUB_S3_BIND_ADDR '0.0.0.0:9000'
    Set-ScopedEnvironmentVariable MEDIAHUB_DATABASE_URL $databaseUrl
    Set-ScopedEnvironmentVariable MEDIAHUB_STORAGE_ROOT '/data/storage'
    Set-ScopedEnvironmentVariable MEDIAHUB_WEB_ROOT '/app/web'
    Set-ScopedEnvironmentVariable MEDIAHUB_ACCESS_KEY_MASTER_KEY $masterKey
    Set-ScopedEnvironmentVariable MEDIAHUB_ACCESS_KEY_MASTER_KEY_VERSION '1'
    Set-ScopedEnvironmentVariable MEDIAHUB_MEDIA_SIGNING_KEY $mediaKey
    Set-ScopedEnvironmentVariable MEDIAHUB_REGISTRATION_ENABLED 'true'
    Set-ScopedEnvironmentVariable MEDIAHUB_EXPOSE_AUTH_TOKENS 'true'
    Set-ScopedEnvironmentVariable MEDIAHUB_ALLOW_INSECURE_COOKIES 'true'
    Set-ScopedEnvironmentVariable MEDIAHUB_COOKIE_SAME_SITE 'lax'
    Set-ScopedEnvironmentVariable PRISMARK_GC_GRACE_HOURS '0'
    if ($Mode -eq 'DockerSilo') {
        Set-ScopedEnvironmentVariable MEDIAHUB_STORAGE_BACKEND 's3'
        Set-ScopedEnvironmentVariable MEDIAHUB_S3_BUCKET $siloBucket
        Set-ScopedEnvironmentVariable MEDIAHUB_S3_REGION $Region
        Set-ScopedEnvironmentVariable MEDIAHUB_S3_ENDPOINT 'http://silo:9000'
        Set-ScopedEnvironmentVariable MEDIAHUB_S3_ACCESS_KEY_ID $siloAccessKey
        Set-ScopedEnvironmentVariable MEDIAHUB_S3_SECRET_ACCESS_KEY $siloSecretKey
        Set-ScopedEnvironmentVariable MEDIAHUB_S3_ALLOW_HTTP 'true'
        Set-ScopedEnvironmentVariable MEDIAHUB_S3_VIRTUAL_HOSTED_STYLE 'false'
    }
    else {
        Set-ScopedEnvironmentVariable MEDIAHUB_STORAGE_BACKEND 'local'
    }

    $prismarkEnvironment = @(
        'MEDIAHUB_BIND_ADDR', 'MEDIAHUB_S3_BIND_ADDR', 'MEDIAHUB_DATABASE_URL',
        'MEDIAHUB_STORAGE_BACKEND', 'MEDIAHUB_STORAGE_ROOT', 'MEDIAHUB_WEB_ROOT',
        'MEDIAHUB_ACCESS_KEY_MASTER_KEY', 'MEDIAHUB_ACCESS_KEY_MASTER_KEY_VERSION',
        'MEDIAHUB_MEDIA_SIGNING_KEY', 'MEDIAHUB_REGISTRATION_ENABLED',
        'MEDIAHUB_EXPOSE_AUTH_TOKENS', 'MEDIAHUB_ALLOW_INSECURE_COOKIES',
        'MEDIAHUB_COOKIE_SAME_SITE', 'PRISMARK_GC_GRACE_HOURS'
    )
    if ($Mode -eq 'DockerSilo') {
        $prismarkEnvironment += @(
            'MEDIAHUB_S3_BUCKET', 'MEDIAHUB_S3_REGION', 'MEDIAHUB_S3_ENDPOINT',
            'MEDIAHUB_S3_ACCESS_KEY_ID', 'MEDIAHUB_S3_SECRET_ACCESS_KEY',
            'MEDIAHUB_S3_ALLOW_HTTP', 'MEDIAHUB_S3_VIRTUAL_HOSTED_STYLE'
        )
    }
    $runArguments = @(
        'run', '--detach', '--rm', '--name', $prismark,
        '--network', $network,
        '--label', "prismark.s3-compat.run=$($script:RunId)",
        '--publish', '127.0.0.1::3000', '--publish', '127.0.0.1::9000'
    )
    foreach ($name in $prismarkEnvironment) {
        $runArguments += @('--env', $name)
    }
    $runArguments += $PrismArkImage
    Invoke-NativeCommand -Command docker -Arguments $runArguments | Out-Null

    $controlPort = Get-DockerPublishedPort -Container $prismark -ContainerPort 3000
    $s3Port = Get-DockerPublishedPort -Container $prismark -ContainerPort 9000
    $controlEndpoint = "http://127.0.0.1:$controlPort"
    $s3Endpoint = "http://127.0.0.1:$s3Port"
    Wait-Until -Description 'PrismArk readiness' -Probe {
        try {
            $response = Invoke-WebRequest -UseBasicParsing -Uri "$controlEndpoint/health/ready" -TimeoutSec 5
            return $response.StatusCode -eq 200
        }
        catch {
            $state = Invoke-NativeCommand -Command docker -Arguments @('inspect', '--format', '{{.State.Running}}', $prismark) -AllowFailure
            if ($state.ExitCode -ne 0 -or $state.Output -ne 'true') {
                throw "PrismArk container stopped: $(Get-ContainerDiagnostic $prismark)"
            }
            return $false
        }
    }
    return [pscustomobject]@{
        ControlEndpoint = $controlEndpoint
        S3Endpoint = $s3Endpoint
        StorageBackend = if ($Mode -eq 'DockerSilo') { 'silo' } else { 'local' }
    }
}

function New-TemporaryPrismArkCredential {
    param([string]$ControlEndpoint)
    $email = "s3-compat-$($script:RunId)@example.invalid"
    $password = New-RandomSecret 30
    Register-Secret $password
    $session = New-Object Microsoft.PowerShell.Commands.WebRequestSession
    $registration = Invoke-RestMethod -UseBasicParsing -Method Post -Uri "$ControlEndpoint/api/v1/auth/register" -ContentType 'application/json' -Body (@{ email = $email; password = $password } | ConvertTo-Json -Compress) -WebSession $session
    Assert-Condition (-not [string]::IsNullOrWhiteSpace([string]$registration.verification_token)) 'development registration did not return a verification token'
    Register-Secret ([string]$registration.verification_token)
    Invoke-RestMethod -UseBasicParsing -Method Post -Uri "$ControlEndpoint/api/v1/auth/verify-email" -ContentType 'application/json' -Body (@{ token = $registration.verification_token } | ConvertTo-Json -Compress) -WebSession $session | Out-Null
    $login = Invoke-RestMethod -UseBasicParsing -Method Post -Uri "$ControlEndpoint/api/v1/auth/login" -ContentType 'application/json' -Body (@{ email = $email; password = $password } | ConvertTo-Json -Compress) -WebSession $session
    $csrfCookie = $session.Cookies.GetCookies([Uri]$ControlEndpoint) | Where-Object { $_.Name -eq 'mediahub_csrf' } | Select-Object -First 1
    Assert-Condition ($null -ne $csrfCookie) 'login did not issue the CSRF cookie'
    Register-Secret $csrfCookie.Value
    $permissions = @(
        'application:read', 'bucket:list', 'bucket:manage', 'media:list',
        'media:read', 'media:upload', 'media:update', 'media:delete', 'webhook:manage'
    )
    $credential = Invoke-RestMethod -UseBasicParsing -Method Post -Uri "$ControlEndpoint/api/v1/applications/$($login.app_id)/access-keys" -ContentType 'application/json' -Headers @{ 'x-csrf-token' = $csrfCookie.Value } -Body (@{ name = 'S3 compatibility matrix'; permissions = $permissions } | ConvertTo-Json -Compress) -WebSession $session
    Register-Secret ([string]$credential.secret_access_key)
    return [pscustomobject]@{
        AccessKeyId = [string]$credential.access_key_id
        SecretAccessKey = [string]$credential.secret_access_key
    }
}

function Stop-IsolatedStack {
    $errors = New-Object System.Collections.Generic.List[string]
    foreach ($key in @('PrismArk', 'Silo', 'Postgres')) {
        $name = $script:DockerResources[$key]
        if ([string]::IsNullOrWhiteSpace([string]$name)) {
            continue
        }
        $remove = Invoke-NativeCommand -Command docker -Arguments @('rm', '--force', $name) -AllowFailure
        if ($remove.ExitCode -ne 0 -and $remove.Output -notmatch '(?i)(No such container|not found)') {
            $errors.Add("failed to remove container ${name}: $($remove.Output)")
        }
    }
    $network = $script:DockerResources.Network
    if (-not [string]::IsNullOrWhiteSpace([string]$network)) {
        $removeNetwork = Invoke-NativeCommand -Command docker -Arguments @('network', 'rm', $network) -AllowFailure
        if ($removeNetwork.ExitCode -ne 0 -and $removeNetwork.Output -notmatch '(?i)(No such network|not found)') {
            $errors.Add("failed to remove network ${network}: $($removeNetwork.Output)")
        }
    }
    if ($errors.Count -gt 0) {
        throw ($errors -join '; ')
    }
}

function Write-CompatibilityReports {
    param(
        [string]$ResolvedEndpoint,
        [hashtable]$DetectedClients,
        [string]$StorageBackend,
        [AllowNull()][string]$FatalError
    )
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
    $finishedAt = [DateTimeOffset]::UtcNow
    $baseline = Get-CompatibilityBaseline
    $counts = [ordered]@{ PASS = 0; SKIP = 0; XFAIL = 0; FAIL = 0 }
    foreach ($item in $script:Results) {
        $counts[$item.status] = [int]$counts[$item.status] + 1
    }
    $clients = @()
    foreach ($name in @('aws', 'mcli', 'rclone')) {
        $client = $DetectedClients[$name]
        $clients += [pscustomobject][ordered]@{
            name = $name
            detected = $null -ne $client
            executable = if ($null -eq $client) { $null } else { [IO.Path]::GetFileName($client.Path) }
            version = if ($null -eq $client) { $null } else { $client.Version }
        }
    }
    $report = [pscustomobject][ordered]@{
        schema_version = 1
        run_id = $script:RunId
        baseline = $baseline
        started_at = $script:StartedAt.ToString('o')
        finished_at = $finishedAt.ToString('o')
        duration_ms = [long]($finishedAt - $script:StartedAt).TotalMilliseconds
        target = [pscustomobject][ordered]@{
            mode = $Target
            endpoint = $ResolvedEndpoint
            region = $Region
            storage_backend = $StorageBackend
            prismark_image = if ($Target -eq 'Endpoint') { $null } else { $PrismArkImage }
            silo_image = if ($Target -eq 'DockerSilo') { $SiloImage } else { $null }
        }
        clients = $clients
        summary = $counts
        fatal_error = if ([string]::IsNullOrWhiteSpace($FatalError)) { $null } else { Protect-Text $FatalError }
        results = [object[]]($script:Results | ForEach-Object { $_ })
    }
    $stamp = $script:StartedAt.ToString('yyyyMMdd-HHmmss')
    $jsonPath = Join-Path $OutputDirectory "s3-compat-$stamp-$($script:RunId).json"
    $markdownPath = Join-Path $OutputDirectory "s3-compat-$stamp-$($script:RunId).md"
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $jsonPath -Encoding UTF8

    $markdown = New-Object System.Collections.Generic.List[string]
    $markdown.Add('# PrismArk S3 client compatibility matrix')
    $markdown.Add('')
    $markdown.Add(('- Run ID: `{0}`' -f $script:RunId))
    $markdown.Add(('- Baseline: `{0}`' -f $baseline))
    $markdown.Add(('- Target: `{0}` / `{1}`' -f $Target, $ResolvedEndpoint))
    $markdown.Add(('- Storage backend: `{0}`' -f $StorageBackend))
    $markdown.Add(('- Time: `{0}` to `{1}`' -f $script:StartedAt.ToString('o'), $finishedAt.ToString('o')))
    $markdown.Add("- Summary: PASS $($counts.PASS) / SKIP $($counts.SKIP) / XFAIL $($counts.XFAIL) / FAIL $($counts.FAIL)")
    if (-not [string]::IsNullOrWhiteSpace($FatalError)) {
        $markdown.Add("- Fatal error: $(Protect-Text $FatalError)")
    }
    $markdown.Add('')
    $markdown.Add('| Client | Operation | Status | Duration (ms) | Detail |')
    $markdown.Add('| --- | --- | --- | ---: | --- |')
    foreach ($item in $script:Results) {
        $message = (Protect-Text $item.message).Replace('|', '\|').Replace("`r", ' ').Replace("`n", ' ')
        $markdown.Add("| $($item.client) | $($item.operation) | $($item.status) | $($item.duration_ms) | $message |")
    }
    $markdown.Add('')
    $markdown.Add('Status definitions: PASS is an asserted success; SKIP means the client is missing or has no stable command for the operation; XFAIL is a confirmed known PrismArk protocol gap; FAIL is an unexpected failure.')
    $markdown | Set-Content -LiteralPath $markdownPath -Encoding UTF8
    return [pscustomobject]@{ Json = $jsonPath; Markdown = $markdownPath; Failures = [int]$counts.FAIL }
}

. (Join-Path $PSScriptRoot 'S3Compat.Clients.ps1')

$detectedClients = @{}
$fatalError = $null
$resolvedEndpoint = if ($null -eq $Endpoint) { '' } else { $Endpoint.TrimEnd('/') }
$storageBackend = if ($Target -eq 'Endpoint') { 'external' } else { '' }
$credential = $null
$reportPaths = $null

try {
    New-Item -ItemType Directory -Path $script:TempRoot -Force | Out-Null
    $detectedClients.aws = Get-NativeClient @('aws')
    $detectedClients.mcli = Get-NativeClient @('mcli', 'mc')
    $detectedClients.rclone = Get-NativeClient @('rclone')
    Initialize-ClientVersions -Clients $detectedClients

    $availableCount = @($detectedClients.Values | Where-Object { $null -ne $_ }).Count
    if ($availableCount -eq 0) {
        Add-MissingClientResults -Clients $detectedClients
        Write-CompatInfo 'no supported host client is installed; Docker startup is skipped'
    }
    else {
        if ($Target -eq 'Endpoint') {
            if ([string]::IsNullOrWhiteSpace($resolvedEndpoint)) {
                throw '-Endpoint is required when Target=Endpoint'
            }
            $accessKeyId = [Environment]::GetEnvironmentVariable('PRISMARK_S3_ACCESS_KEY_ID', 'Process')
            $secretAccessKey = [Environment]::GetEnvironmentVariable('PRISMARK_S3_SECRET_ACCESS_KEY', 'Process')
            if ([string]::IsNullOrWhiteSpace($accessKeyId) -or [string]::IsNullOrWhiteSpace($secretAccessKey)) {
                throw 'Endpoint mode requires PRISMARK_S3_ACCESS_KEY_ID and PRISMARK_S3_SECRET_ACCESS_KEY'
            }
            Register-Secret $secretAccessKey
            $credential = [pscustomobject]@{ AccessKeyId = $accessKeyId; SecretAccessKey = $secretAccessKey }
            Add-CompatResult -Client harness -Operation Environment.Setup -Status PASS -Message 'using caller-provided endpoint; no Docker resources created'
        }
        else {
            $stack = Start-IsolatedStack -Mode $Target
            $resolvedEndpoint = $stack.S3Endpoint
            $storageBackend = $stack.StorageBackend
            $credential = New-TemporaryPrismArkCredential -ControlEndpoint $stack.ControlEndpoint
            Add-CompatResult -Client harness -Operation Environment.Setup -Status PASS -Message "isolated $Target stack is ready"
        }

        Invoke-DetectedClientMatrices -Clients $detectedClients -Endpoint $resolvedEndpoint -Region $Region -Credential $credential -TempRoot $script:TempRoot -RunId $script:RunId
    }
}
catch {
    $fatalError = Protect-Text $_.Exception.Message
    Add-CompatResult -Client harness -Operation Environment.Run -Status FAIL -Message $fatalError
    if ($Target -ne 'Endpoint' -and -not [string]::IsNullOrWhiteSpace([string]$script:DockerResources.PrismArk)) {
        $diagnostic = Get-ContainerDiagnostic $script:DockerResources.PrismArk
        if (-not [string]::IsNullOrWhiteSpace($diagnostic)) {
            Write-CompatInfo "PrismArk diagnostic: $diagnostic"
        }
    }
}
finally {
    if ($Target -ne 'Endpoint' -and -not [string]::IsNullOrWhiteSpace([string]$script:DockerResources.Network)) {
        try {
            Stop-IsolatedStack
            Add-CompatResult -Client harness -Operation Environment.Cleanup -Status PASS -Message 'all disposable containers and the isolated network were removed'
        }
        catch {
            Add-CompatResult -Client harness -Operation Environment.Cleanup -Status FAIL -Message $_.Exception.Message
        }
    }
    Restore-ScopedEnvironmentVariables
    if (Test-Path -LiteralPath $script:TempRoot) {
        $resolvedTemp = [IO.Path]::GetFullPath($script:TempRoot)
        $systemTemp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
        if ($resolvedTemp.StartsWith($systemTemp, [StringComparison]::OrdinalIgnoreCase) -and (Split-Path $resolvedTemp -Leaf) -like 'prismark-s3-compat-*') {
            Remove-Item -LiteralPath $resolvedTemp -Recurse -Force
        }
        else {
            Add-CompatResult -Client harness -Operation TemporaryFiles.Cleanup -Status FAIL -Message "refused to remove unexpected temp path: $resolvedTemp"
        }
    }
    $reportPaths = Write-CompatibilityReports -ResolvedEndpoint $resolvedEndpoint -DetectedClients $detectedClients -StorageBackend $storageBackend -FatalError $fatalError
}

Write-CompatInfo "JSON report: $($reportPaths.Json)"
Write-CompatInfo "Markdown report: $($reportPaths.Markdown)"
if ($reportPaths.Failures -gt 0) {
    exit 1
}
exit 0
