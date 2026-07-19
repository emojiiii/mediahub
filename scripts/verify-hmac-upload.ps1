param(
    [string]$ServerExecutable = "target\debug\mediahub-server.exe",
    [string]$DatabaseUrl = $env:MEDIAHUB_TEST_POSTGRES_URL
)

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($DatabaseUrl)) {
    throw "Set MEDIAHUB_TEST_POSTGRES_URL to an isolated PostgreSQL test database"
}
$workspace = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = $listener.LocalEndpoint.Port
$listener.Stop()
$runId = [guid]::NewGuid().ToString("N")
$data = [IO.Path]::GetFullPath((Join-Path $workspace "data\e2e-hmac-$runId"))
if (-not $data.StartsWith($workspace, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Unsafe temporary path"
}
New-Item -ItemType Directory -Path $data -Force | Out-Null

$env:MEDIAHUB_BIND_ADDR = "127.0.0.1:$port"
$env:MEDIAHUB_DATABASE_URL = $DatabaseUrl
$env:MEDIAHUB_STORAGE_ROOT = Join-Path $data "storage"
$env:MEDIAHUB_ACCESS_KEY_MASTER_KEY = [Convert]::ToBase64String(
    [byte[]](0..31 | ForEach-Object { 65 })
)
$env:MEDIAHUB_MEDIA_SIGNING_KEY = [Convert]::ToBase64String(
    [byte[]](0..31 | ForEach-Object { 66 })
)
$env:MEDIAHUB_ALLOW_INSECURE_COOKIES = "true"

function ConvertTo-LowerHex([byte[]]$Bytes) {
    ([BitConverter]::ToString($Bytes)).Replace("-", "").ToLowerInvariant()
}

function New-SignedHeaders(
    [string]$Body,
    [string]$Nonce,
    [string]$IdempotencyKey,
    $Key
) {
    $date = [DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ssZ")
    $bodyBytes = [Text.Encoding]::UTF8.GetBytes($Body)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $bodyHash = ConvertTo-LowerHex $sha.ComputeHash($bodyBytes)
    }
    finally {
        $sha.Dispose()
    }
    $signedNames = @(
        "idempotency-key"
        "x-mediahub-access-key"
        "x-mediahub-content-sha256"
        "x-mediahub-date"
        "x-mediahub-nonce"
    ) -join ","
    $canonicalHeaders = @(
        "idempotency-key:$IdempotencyKey"
        "x-mediahub-access-key:$($Key.access_key_id)"
        "x-mediahub-content-sha256:$bodyHash"
        "x-mediahub-date:$date"
        "x-mediahub-nonce:$Nonce"
    ) -join "`n"
    $canonical = @(
        "POST"
        "/api/v1/uploads"
        ""
        $canonicalHeaders
        $bodyHash
        $date
        $Nonce
        $IdempotencyKey
    ) -join "`n"
    $secretBytes = [Text.Encoding]::UTF8.GetBytes([string]$Key.secret_access_key)
    $hmac = [Security.Cryptography.HMACSHA256]::new($secretBytes)
    try {
        $signature = ConvertTo-LowerHex $hmac.ComputeHash(
            [Text.Encoding]::UTF8.GetBytes($canonical)
        )
    }
    finally {
        $hmac.Dispose()
    }
    @{
        "X-MediaHub-Access-Key" = [string]$Key.access_key_id
        "X-MediaHub-Date" = $date
        "X-MediaHub-Content-SHA256" = $bodyHash
        "X-MediaHub-Nonce" = $Nonce
        "Idempotency-Key" = $IdempotencyKey
        "Authorization" = "MH-HMAC-SHA256 SignedHeaders=$signedNames; Signature=$signature"
    }
}

$stdout = Join-Path $data "stdout.log"
$stderr = Join-Path $data "stderr.log"
$executable = [IO.Path]::GetFullPath((Join-Path $workspace $ServerExecutable))
$server = Start-Process $executable -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput $stdout -RedirectStandardError $stderr
try {
    $base = "http://127.0.0.1:$port"
    $ready = $false
    foreach ($attempt in 1..40) {
        if ($server.HasExited) {
            break
        }
        try {
            $probe = Invoke-WebRequest "$base/health/ready" -UseBasicParsing -TimeoutSec 1
            if ($probe.StatusCode -eq 200) {
                $ready = $true
                break
            }
        }
        catch {
            Start-Sleep -Milliseconds 200
        }
    }
    if (-not $ready) {
        throw (Get-Content -Raw $stderr)
    }

    $registration = @{
        email = "hmac-$runId@example.com"
        password = "CorrectHorseBatteryStaple123!"
    } | ConvertTo-Json -Compress
    $register = Invoke-RestMethod "$base/api/v1/auth/register" -Method Post `
        -ContentType "application/json" -Body $registration -SessionVariable session
    $csrf = $session.Cookies.GetCookies([uri]$base)["mediahub_csrf"].Value
    $accessBody = @{
        name = "Upload E2E"
        permissions = @("media:upload")
    } | ConvertTo-Json -Compress
    $key = Invoke-RestMethod `
        "$base/api/v1/applications/$($register.app_id)/access-keys" `
        -Method Post -ContentType "application/json" -Body $accessBody `
        -Headers @{ "X-CSRF-Token" = $csrf } -WebSession $session

    $body = @{
        bucket = "media"
        object_key = "hmac/$runId.txt"
        expected_size = 5
        content_type = "text/plain"
    } | ConvertTo-Json -Compress
    $idempotencyKey = "upload-$runId"
    $first = Invoke-WebRequest "$base/api/v1/uploads" -Method Post `
        -ContentType "application/json" -Body $body `
        -Headers (New-SignedHeaders $body "nonce-1-$runId" $idempotencyKey $key) `
        -UseBasicParsing
    $second = Invoke-WebRequest "$base/api/v1/uploads" -Method Post `
        -ContentType "application/json" -Body $body `
        -Headers (New-SignedHeaders $body "nonce-2-$runId" $idempotencyKey $key) `
        -UseBasicParsing
    if ($first.StatusCode -ne 201 -or $second.StatusCode -ne 201) {
        throw "HMAC upload creation did not return 201/201"
    }
    if ($first.Content -cne $second.Content) {
        throw "HMAC idempotency replay did not return identical bytes"
    }
    $created = $first.Content | ConvertFrom-Json
    $quota = Invoke-RestMethod "$base/api/v1/me" -WebSession $session
    if ($quota.reserved_bytes -ne 5) {
        throw "HMAC replay reserved quota more than once"
    }

    $changedBody = @{
        bucket = "media"
        object_key = "hmac/$runId-other.txt"
        expected_size = 5
        content_type = "text/plain"
    } | ConvertTo-Json -Compress
    try {
        $conflictStatus = (Invoke-WebRequest "$base/api/v1/uploads" -Method Post `
            -ContentType "application/json" -Body $changedBody `
            -Headers (New-SignedHeaders $changedBody "nonce-3-$runId" $idempotencyKey $key) `
            -UseBasicParsing).StatusCode
    }
    catch {
        $conflictStatus = [int]$_.Exception.Response.StatusCode
    }
    if ($conflictStatus -ne 409) {
        throw "Changed idempotency request did not return 409"
    }

    [pscustomobject]@{
        upload_id = $created.upload_id
        first = $first.StatusCode
        replay = $second.StatusCode
        response_bytes_equal = $first.Content -ceq $second.Content
        reserved_bytes = $quota.reserved_bytes
        changed_request = $conflictStatus
    } | ConvertTo-Json -Compress
}
finally {
    if ($server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force
        $server.WaitForExit()
    }
    if (Test-Path -LiteralPath $data) {
        Remove-Item -LiteralPath $data -Recurse -Force
    }
}
