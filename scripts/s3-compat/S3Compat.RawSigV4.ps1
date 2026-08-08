#requires -Version 5.1

Set-StrictMode -Version Latest

$script:S3RawUtf8 = New-Object System.Text.UTF8Encoding($false)

function ConvertTo-S3RawAwsUriEncoded {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value,
        [switch]$PreserveSlash
    )

    $builder = New-Object System.Text.StringBuilder
    foreach ($octet in $script:S3RawUtf8.GetBytes($Value)) {
        $isUnreserved =
            ($octet -ge 0x41 -and $octet -le 0x5A) -or
            ($octet -ge 0x61 -and $octet -le 0x7A) -or
            ($octet -ge 0x30 -and $octet -le 0x39) -or
            $octet -eq 0x2D -or $octet -eq 0x2E -or $octet -eq 0x5F -or $octet -eq 0x7E
        if ($isUnreserved -or ($PreserveSlash -and $octet -eq 0x2F)) {
            [void]$builder.Append([char]$octet)
        }
        else {
            [void]$builder.Append(('%{0:X2}' -f $octet))
        }
    }
    return $builder.ToString()
}

function Get-S3RawSha256Hex {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][byte[]]$Bytes)

    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($algorithm.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
    }
}

function Get-S3RawMd5Base64 {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][byte[]]$Bytes)

    $algorithm = [Security.Cryptography.MD5]::Create()
    try {
        return [Convert]::ToBase64String($algorithm.ComputeHash($Bytes))
    }
    finally {
        $algorithm.Dispose()
    }
}

function Get-S3RawHmacSha256 {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][byte[]]$Key,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value
    )

    $algorithm = New-Object Security.Cryptography.HMACSHA256 -ArgumentList (, $Key)
    try {
        return $algorithm.ComputeHash($script:S3RawUtf8.GetBytes($Value))
    }
    finally {
        $algorithm.Dispose()
    }
}

function Get-S3RawCanonicalQuery {
    param([object[]]$Parameters = @())

    $encoded = New-Object System.Collections.Generic.List[string]
    foreach ($parameter in @($Parameters)) {
        if ($null -eq $parameter) {
            throw 'raw SigV4 query parameters cannot contain null entries'
        }
        $nameProperty = $parameter.PSObject.Properties['Name']
        $valueProperty = $parameter.PSObject.Properties['Value']
        if ($null -eq $nameProperty -or $null -eq $valueProperty) {
            throw 'each raw SigV4 query parameter must contain Name and Value'
        }
        $name = ConvertTo-S3RawAwsUriEncoded -Value ([string]$nameProperty.Value)
        $value = ConvertTo-S3RawAwsUriEncoded -Value ([string]$valueProperty.Value)
        $encoded.Add("${name}=${value}")
    }
    $encoded.Sort([StringComparer]::Ordinal)
    return $encoded.ToArray() -join '&'
}

function Get-S3RawCanonicalHeaders {
    param([Parameter(Mandatory = $true)][System.Collections.IDictionary]$Headers)

    $normalized = @{}
    foreach ($entry in $Headers.GetEnumerator()) {
        $name = ([string]$entry.Key).ToLowerInvariant()
        if ($name -notmatch '^[a-z0-9-]+$') {
            throw "raw SigV4 header name is invalid: $name"
        }
        if ($normalized.ContainsKey($name)) {
            throw "raw SigV4 header occurs more than once: $name"
        }
        $value = [string]$entry.Value
        if ($value -match '[\x00-\x08\x0A-\x1F\x7F]') {
            throw "raw SigV4 header contains an invalid control character: $name"
        }
        $normalized[$name] = [regex]::Replace($value.Trim(), '[\x20\x09]+', ' ')
    }

    $names = [string[]]@($normalized.Keys)
    [Array]::Sort($names, [StringComparer]::Ordinal)
    $canonical = New-Object System.Text.StringBuilder
    foreach ($name in $names) {
        [void]$canonical.Append($name).Append(':').Append($normalized[$name]).Append("`n")
    }
    return [pscustomobject]@{
        Values = $normalized
        SignedHeaders = $names -join ';'
        CanonicalHeaders = $canonical.ToString()
    }
}

function New-S3RawSigV4Signature {
    param(
        [Parameter(Mandatory = $true)][ValidatePattern('^[A-Z]+$')][string]$Method,
        [Parameter(Mandatory = $true)][string]$CanonicalUri,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$CanonicalQuery,
        [Parameter(Mandatory = $true)][System.Collections.IDictionary]$Headers,
        [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{64}$')][string]$PayloadHash,
        [Parameter(Mandatory = $true)][string]$AccessKeyId,
        [Parameter(Mandatory = $true)][string]$SecretAccessKey,
        [Parameter(Mandatory = $true)][ValidatePattern('^[a-z0-9][a-z0-9-]{0,62}$')][string]$Region,
        [Parameter(Mandatory = $true)][DateTimeOffset]$Timestamp
    )

    if ([string]::IsNullOrWhiteSpace($AccessKeyId) -or [string]::IsNullOrWhiteSpace($SecretAccessKey)) {
        throw 'raw SigV4 credentials must not be empty'
    }
    if ($AccessKeyId -match '[\x00-\x20\x7F]' -or $SecretAccessKey -match '[\x00-\x1F\x7F]') {
        throw 'raw SigV4 credentials contain invalid control or whitespace characters'
    }
    if (-not $CanonicalUri.StartsWith('/')) {
        throw 'raw SigV4 canonical URI must be absolute'
    }

    $canonicalHeaders = Get-S3RawCanonicalHeaders -Headers $Headers
    $amzDate = $Timestamp.UtcDateTime.ToString('yyyyMMddTHHmmssZ', [Globalization.CultureInfo]::InvariantCulture)
    $dateStamp = $Timestamp.UtcDateTime.ToString('yyyyMMdd', [Globalization.CultureInfo]::InvariantCulture)
    $headerDate = [string]$canonicalHeaders.Values['x-amz-date']
    if ($headerDate -ne $amzDate) {
        throw 'raw SigV4 x-amz-date does not match the signing timestamp'
    }

    $canonicalRequest = @(
        $Method
        $CanonicalUri
        $CanonicalQuery
        $canonicalHeaders.CanonicalHeaders
        $canonicalHeaders.SignedHeaders
        $PayloadHash
    ) -join "`n"
    $scope = "${dateStamp}/${Region}/s3/aws4_request"
    $stringToSign = @(
        'AWS4-HMAC-SHA256'
        $amzDate
        $scope
        (Get-S3RawSha256Hex -Bytes $script:S3RawUtf8.GetBytes($canonicalRequest))
    ) -join "`n"
    $dateKey = Get-S3RawHmacSha256 -Key $script:S3RawUtf8.GetBytes("AWS4${SecretAccessKey}") -Value $dateStamp
    $regionKey = Get-S3RawHmacSha256 -Key $dateKey -Value $Region
    $serviceKey = Get-S3RawHmacSha256 -Key $regionKey -Value 's3'
    $signingKey = Get-S3RawHmacSha256 -Key $serviceKey -Value 'aws4_request'
    $signature = ([BitConverter]::ToString((Get-S3RawHmacSha256 -Key $signingKey -Value $stringToSign))).Replace('-', '').ToLowerInvariant()
    $authorization = "AWS4-HMAC-SHA256 Credential=${AccessKeyId}/${scope},SignedHeaders=$($canonicalHeaders.SignedHeaders),Signature=${signature}"

    return [pscustomobject]@{
        Authorization = $authorization
        Signature = $signature
        SignedHeaders = $canonicalHeaders.SignedHeaders
    }
}

function Resolve-S3RawEndpoint {
    param([Parameter(Mandatory = $true)][string]$Endpoint)

    $uri = $null
    if (-not [Uri]::TryCreate($Endpoint, [UriKind]::Absolute, [ref]$uri)) {
        throw 'raw SigV4 endpoint must be an absolute URI'
    }
    if ($uri.Scheme -ne 'http' -and $uri.Scheme -ne 'https') {
        throw 'raw SigV4 endpoint scheme must be http or https'
    }
    if ([string]::IsNullOrWhiteSpace($uri.Host)) {
        throw 'raw SigV4 endpoint must contain a host'
    }
    if (-not [string]::IsNullOrEmpty($uri.UserInfo)) {
        throw 'raw SigV4 endpoint must not contain userinfo'
    }
    if (-not [string]::IsNullOrEmpty($uri.Fragment)) {
        throw 'raw SigV4 endpoint must not contain a fragment'
    }
    if (-not [string]::IsNullOrEmpty($uri.Query)) {
        throw 'raw SigV4 endpoint must not contain a query string'
    }
    return $uri
}

function New-S3RawRequestParts {
    param(
        [Parameter(Mandatory = $true)][ValidatePattern('^[A-Z]+$')][string]$Method,
        [Parameter(Mandatory = $true)][string]$Endpoint,
        [Parameter(Mandatory = $true)][ValidatePattern('^[a-z0-9][a-z0-9.-]{1,62}$')][string]$Bucket,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Key,
        [object[]]$QueryParameters = @(),
        [System.Collections.IDictionary]$AdditionalHeaders,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][byte[]]$Body,
        [string]$ContentType,
        [Parameter(Mandatory = $true)][string]$AccessKeyId,
        [Parameter(Mandatory = $true)][string]$SecretAccessKey,
        [Parameter(Mandatory = $true)][string]$Region,
        [DateTimeOffset]$Timestamp = [DateTimeOffset]::UtcNow
    )

    if ($Body.Length -gt 1048576) {
        throw 'raw SigV4 request body exceeds the 1 MiB safety limit'
    }
    if ($Key -match '[\x00-\x1F\x7F]') {
        throw 'raw SigV4 object key contains a control character'
    }
    $endpointUri = Resolve-S3RawEndpoint -Endpoint $Endpoint
    $basePath = [Uri]::UnescapeDataString($endpointUri.AbsolutePath).TrimEnd('/')
    if ($basePath -eq '/') {
        $basePath = ''
    }
    $resourcePath = "${basePath}/${Bucket}"
    if (-not [string]::IsNullOrEmpty($Key)) {
        $resourcePath = "${resourcePath}/${Key}"
    }
    $canonicalUri = ConvertTo-S3RawAwsUriEncoded -Value $resourcePath -PreserveSlash
    $canonicalQuery = Get-S3RawCanonicalQuery -Parameters $QueryParameters
    $uriText = $endpointUri.GetLeftPart([UriPartial]::Authority) + $canonicalUri
    if (-not [string]::IsNullOrEmpty($canonicalQuery)) {
        $uriText = "${uriText}?${canonicalQuery}"
    }
    $requestUri = New-Object Uri $uriText
    if ($requestUri.AbsolutePath -ne $canonicalUri -or $requestUri.Query.TrimStart('?') -ne $canonicalQuery) {
        throw 'raw SigV4 request URI was normalized and can no longer be signed safely'
    }

    $payloadHash = Get-S3RawSha256Hex -Bytes $Body
    $amzDate = $Timestamp.UtcDateTime.ToString('yyyyMMddTHHmmssZ', [Globalization.CultureInfo]::InvariantCulture)
    $headers = [ordered]@{
        host = $requestUri.Authority.ToLowerInvariant()
        'content-md5' = Get-S3RawMd5Base64 -Bytes $Body
        'x-amz-content-sha256' = $payloadHash
        'x-amz-date' = $amzDate
    }
    if (-not [string]::IsNullOrWhiteSpace($ContentType)) {
        $headers['content-type'] = $ContentType.Trim()
    }
    if ($null -ne $AdditionalHeaders) {
        foreach ($entry in $AdditionalHeaders.GetEnumerator()) {
            $name = ([string]$entry.Key).ToLowerInvariant()
            if ($name -in @('authorization', 'host', 'content-length', 'content-md5', 'content-type', 'x-amz-content-sha256', 'x-amz-date')) {
                throw "raw SigV4 additional header cannot override managed header: $name"
            }
            if ($headers.Contains($name)) {
                throw "raw SigV4 additional header occurs more than once: $name"
            }
            $headers[$name] = [string]$entry.Value
        }
    }

    $signed = New-S3RawSigV4Signature -Method $Method -CanonicalUri $canonicalUri -CanonicalQuery $canonicalQuery -Headers $headers -PayloadHash $payloadHash -AccessKeyId $AccessKeyId -SecretAccessKey $SecretAccessKey -Region $Region -Timestamp $Timestamp
    $headers['authorization'] = $signed.Authorization
    return [pscustomobject]@{
        Uri = $requestUri
        Headers = $headers
    }
}

function Protect-S3RawDiagnosticText {
    param(
        [AllowNull()][object]$Value,
        [Parameter(Mandatory = $true)][string]$SecretAccessKey
    )

    if ($null -eq $Value) {
        return ''
    }
    $safe = [string]$Value
    if (-not [string]::IsNullOrEmpty($SecretAccessKey)) {
        $safe = $safe.Replace($SecretAccessKey, '<redacted>')
        $escapedSecret = [Uri]::EscapeDataString($SecretAccessKey)
        if ($escapedSecret -ne $SecretAccessKey) {
            $safe = $safe.Replace($escapedSecret, '<redacted>')
        }
    }
    return [regex]::Replace(
        $safe,
        '(?i)AWS4-HMAC-SHA256\s+Credential=[^\r\n<]+',
        '<redacted-authorization>'
    )
}

function Read-S3RawBoundedBody {
    param(
        [Parameter(Mandatory = $true)][IO.Stream]$Stream,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65536)][int]$MaximumBytes
    )

    $memory = New-Object IO.MemoryStream
    $buffer = New-Object byte[] 4096
    $truncated = $false
    try {
        while ($memory.Length -le $MaximumBytes) {
            $remaining = ($MaximumBytes + 1) - [int]$memory.Length
            $read = $Stream.Read($buffer, 0, [Math]::Min($buffer.Length, $remaining))
            if ($read -le 0) {
                break
            }
            $memory.Write($buffer, 0, $read)
        }
        $bytes = $memory.ToArray()
        if ($bytes.Length -gt $MaximumBytes) {
            $truncated = $true
            $bounded = New-Object byte[] $MaximumBytes
            [Array]::Copy($bytes, $bounded, $MaximumBytes)
            $bytes = $bounded
        }
        return [pscustomobject]@{
            Text = $script:S3RawUtf8.GetString($bytes)
            Truncated = $truncated
        }
    }
    finally {
        $memory.Dispose()
    }
}

function Get-S3RawBoundedResponseHeaders {
    param(
        [Parameter(Mandatory = $true)][Net.WebHeaderCollection]$Source,
        [Parameter(Mandatory = $true)][ValidateRange(1024, 32768)][int]$MaximumCharacters,
        [Parameter(Mandatory = $true)][string]$SecretAccessKey
    )

    $headers = [ordered]@{}
    $used = 0
    $truncated = $false
    foreach ($rawName in $Source.AllKeys) {
        $name = ([string]$rawName).ToLowerInvariant()
        $value = Protect-S3RawDiagnosticText -Value ([string]$Source[$rawName]) -SecretAccessKey $SecretAccessKey
        if ($name.Length -gt 128) {
            $name = $name.Substring(0, 128)
            $truncated = $true
        }
        if ($value.Length -gt 2048) {
            $value = $value.Substring(0, 2048)
            $truncated = $true
        }
        if ($used + $name.Length + $value.Length -gt $MaximumCharacters) {
            $truncated = $true
            break
        }
        $headers[$name] = $value
        $used += $name.Length + $value.Length
    }
    return [pscustomobject]@{ Values = $headers; Truncated = $truncated }
}

function Invoke-S3RawSigV4Request {
    param(
        [Parameter(Mandatory = $true)][ValidateSet('GET', 'HEAD', 'PUT', 'POST', 'DELETE')][string]$Method,
        [Parameter(Mandatory = $true)][string]$Endpoint,
        [Parameter(Mandatory = $true)][string]$Bucket,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Key,
        [object[]]$QueryParameters = @(),
        [System.Collections.IDictionary]$AdditionalHeaders,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][byte[]]$Body,
        [string]$ContentType,
        [Parameter(Mandatory = $true)][string]$AccessKeyId,
        [Parameter(Mandatory = $true)][string]$SecretAccessKey,
        [Parameter(Mandatory = $true)][string]$Region,
        [ValidateRange(1, 60)][int]$TimeoutSeconds = 15,
        [ValidateRange(1, 65536)][int]$MaximumResponseBodyBytes = 32768,
        [ValidateRange(1024, 32768)][int]$MaximumResponseHeaderCharacters = 16384
    )

    $parts = New-S3RawRequestParts -Method $Method -Endpoint $Endpoint -Bucket $Bucket -Key $Key -QueryParameters $QueryParameters -AdditionalHeaders $AdditionalHeaders -Body $Body -ContentType $ContentType -AccessKeyId $AccessKeyId -SecretAccessKey $SecretAccessKey -Region $Region
    $request = [Net.HttpWebRequest]::Create($parts.Uri)
    $request.Method = $Method
    $request.AllowAutoRedirect = $false
    $request.KeepAlive = $false
    $request.Timeout = $TimeoutSeconds * 1000
    $request.ReadWriteTimeout = $TimeoutSeconds * 1000
    $request.ContentLength = $Body.Length
    $request.Host = [string]$parts.Headers['host']
    $request.ServicePoint.Expect100Continue = $false
    foreach ($entry in $parts.Headers.GetEnumerator()) {
        $name = ([string]$entry.Key).ToLowerInvariant()
        switch ($name) {
            'host' { continue }
            'content-type' { $request.ContentType = [string]$entry.Value; continue }
            default { $request.Headers[[string]$entry.Key] = [string]$entry.Value }
        }
    }

    $watch = [Diagnostics.Stopwatch]::StartNew()
    $response = $null
    try {
        if ($Body.Length -gt 0) {
            $requestStream = $request.GetRequestStream()
            try {
                $requestStream.Write($Body, 0, $Body.Length)
            }
            finally {
                $requestStream.Dispose()
            }
        }
        try {
            $response = [Net.HttpWebResponse]$request.GetResponse()
        }
        catch [Net.WebException] {
            if ($null -eq $_.Exception.Response) {
                $watch.Stop()
                $status = [string]$_.Exception.Status
                throw "raw SigV4 request failed before receiving an HTTP response: $status"
            }
            $response = [Net.HttpWebResponse]$_.Exception.Response
        }

        $bodyResult = $null
        $stream = $response.GetResponseStream()
        try {
            if ($null -eq $stream) {
                $bodyResult = [pscustomobject]@{ Text = ''; Truncated = $false }
            }
            else {
                $bodyResult = Read-S3RawBoundedBody -Stream $stream -MaximumBytes $MaximumResponseBodyBytes
            }
        }
        finally {
            if ($null -ne $stream) {
                $stream.Dispose()
            }
        }
        $headerResult = Get-S3RawBoundedResponseHeaders -Source $response.Headers -MaximumCharacters $MaximumResponseHeaderCharacters -SecretAccessKey $SecretAccessKey
        $watch.Stop()
        return [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            StatusDescription = (Protect-S3RawDiagnosticText -Value $response.StatusDescription -SecretAccessKey $SecretAccessKey)
            Headers = $headerResult.Values
            HeadersTruncated = [bool]$headerResult.Truncated
            Body = (Protect-S3RawDiagnosticText -Value $bodyResult.Text -SecretAccessKey $SecretAccessKey)
            BodyTruncated = [bool]$bodyResult.Truncated
            DurationMs = $watch.ElapsedMilliseconds
            VersionId = (Protect-S3RawDiagnosticText -Value ([string]$response.Headers['x-amz-version-id']) -SecretAccessKey $SecretAccessKey)
        }
    }
    catch {
        $watch.Stop()
        $message = Protect-S3RawDiagnosticText -Value $_.Exception.Message -SecretAccessKey $SecretAccessKey
        throw $message
    }
    finally {
        if ($null -ne $response) {
            $response.Dispose()
        }
    }
}

function Test-S3RawSigV4Golden {
    $emptyHash = 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'
    $timestamp = [DateTimeOffset]::ParseExact(
        '20130524T000000Z',
        'yyyyMMddTHHmmssZ',
        [Globalization.CultureInfo]::InvariantCulture,
        [Globalization.DateTimeStyles]::AssumeUniversal
    )
    $headers = [ordered]@{
        host = 'examplebucket.s3.amazonaws.com'
        range = 'bytes=0-9'
        'x-amz-content-sha256' = $emptyHash
        'x-amz-date' = '20130524T000000Z'
    }
    $signed = New-S3RawSigV4Signature -Method GET -CanonicalUri '/test.txt' -CanonicalQuery '' -Headers $headers -PayloadHash $emptyHash -AccessKeyId 'AKIAIOSFODNN7EXAMPLE' -SecretAccessKey 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY' -Region 'us-east-1' -Timestamp $timestamp
    if ($signed.Signature -ne 'f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41') {
        throw 'raw SigV4 signature differs from the published AWS S3 GET Object golden'
    }
    if ($signed.SignedHeaders -ne 'host;range;x-amz-content-sha256;x-amz-date') {
        throw 'raw SigV4 signed header ordering differs from the AWS golden'
    }
    if ((Get-S3RawMd5Base64 -Bytes $script:S3RawUtf8.GetBytes('abc')) -ne 'kAFQmDzST7DWlj99KOF/cg==') {
        throw 'raw SigV4 Content-MD5 primitive failed its offline golden'
    }
    $query = Get-S3RawCanonicalQuery -Parameters @(
        [pscustomobject]@{ Name = 'versionId'; Value = 'a/b' },
        [pscustomobject]@{ Name = 'tagging'; Value = '' }
    )
    if ($query -ne 'tagging=&versionId=a%2Fb') {
        throw 'raw SigV4 canonical query sorting or encoding failed its offline golden'
    }
    $unicodeKey = 'folder/' + [char]0x7A7A + ' ' + [char]0x683C + '.txt'
    $parts = New-S3RawRequestParts -Method PUT -Endpoint 'http://127.0.0.1:9000/base/' -Bucket 'example-bucket' -Key $unicodeKey -QueryParameters @([pscustomobject]@{ Name = 'tagging'; Value = '' }) -AdditionalHeaders @{} -Body $script:S3RawUtf8.GetBytes('abc') -ContentType 'application/xml' -AccessKeyId 'AKIATEST' -SecretAccessKey 'test-secret' -Region 'us-east-1' -Timestamp $timestamp
    if ($parts.Uri.AbsolutePath -ne '/base/example-bucket/folder/%E7%A9%BA%20%E6%A0%BC.txt') {
        throw 'raw SigV4 path-style canonical URI failed its offline golden'
    }
    $boundedStream = New-Object IO.MemoryStream -ArgumentList (, (New-Object byte[] 32))
    try {
        $boundedBody = Read-S3RawBoundedBody -Stream $boundedStream -MaximumBytes 16
    }
    finally {
        $boundedStream.Dispose()
    }
    if (-not $boundedBody.Truncated -or $boundedBody.Text.Length -ne 16) {
        throw 'raw SigV4 bounded response body check failed its offline golden'
    }
    $diagnostic = Protect-S3RawDiagnosticText -Value 'secret AWS4-HMAC-SHA256 Credential=AKIATEST/scope,SignedHeaders=host,Signature=abcd' -SecretAccessKey 'secret'
    if ($diagnostic -match 'secret|Credential=|Signature=') {
        throw 'raw SigV4 diagnostic redaction failed its offline golden'
    }
    return $true
}
