Set-StrictMode -Version Latest

$script:AwsOperations = @(
    'Bucket.Create', 'Bucket.Head', 'Bucket.List', 'Object.Put', 'Object.Get',
    'Object.Head', 'Object.Range', 'Object.IfMatch', 'Object.IfNoneMatch',
    'CopyObject', 'UploadPartCopy',
    'ListObjectsV2.PrefixDelimiter', 'ListObjectsV2.Pagination', 'DeleteObjects',
    'Versioning.NullVersion', 'Versioning.Enable', 'Versioning.ExactRead',
    'Versioning.DeleteMarker', 'Versioning.ExactDelete', 'Multipart.Create',
    'Multipart.UploadPart', 'Multipart.ListParts', 'Multipart.Complete',
    'Multipart.Abort', 'MultipartUploads.List', 'ObjectVersions.List',
    'Tagging.PutObject', 'Tagging.PutObjectTagging', 'Tagging.TagCount',
    'Tagging.VersionIsolation', 'Tagging.CopyDefault', 'Tagging.CopyReplace',
    'Tagging.DeleteObjectTagging', 'Tagging.InvalidTag.Negative',
    'Tagging.DuplicateKey.Negative', 'Tagging.TooMany.Negative',
    'Tagging.BadPercentEncoding.Negative', 'Tagging.Cleanup',
    'ObjectLock.Bucket.Create', 'ObjectLock.Bucket.GetConfiguration',
    'ObjectLock.Bucket.PutDefaultGovernance', 'ObjectLock.Object.PutExplicit',
    'ObjectLock.Object.GetRetention', 'ObjectLock.Object.GetLegalHold',
    'ObjectLock.Object.LegalHoldOff', 'ObjectLock.Governance.DeleteWithoutBypassDenied',
    'ObjectLock.Governance.UnsignedBypassDenied', 'ObjectLock.Governance.BypassDelete',
    'ObjectLock.Bucket.Delete', 'Bucket.Delete'
)

$script:McliOperations = @(
    'Bucket.Create', 'Bucket.Head', 'Bucket.List', 'Object.Put', 'Object.Get',
    'Object.Head', 'Object.Range', 'Object.Conditional', 'CopyObject', 'UploadPartCopy',
    'ListObjectsV2.PrefixDelimiter', 'ListObjectsV2.Pagination', 'DeleteObjects',
    'Versioning.Enable', 'Versioning.NullVersion', 'Versioning.DeleteMarker',
    'Versioning.ExactVersion', 'Multipart.LowLevel', 'MultipartUploads.List',
    'ObjectVersions.List', 'Object.Delete', 'Bucket.Delete'
)

$script:RcloneOperations = @(
    'Bucket.Create', 'Bucket.Head', 'Bucket.List', 'Object.Put', 'Object.Get',
    'Object.Head', 'Object.Range', 'Object.Conditional', 'CopyObject', 'UploadPartCopy',
    'ListObjectsV2.PrefixDelimiter', 'ListObjectsV2.Pagination', 'DeleteObjects',
    'Versioning.Enable', 'Versioning.NullVersion', 'Versioning.DeleteMarker',
    'Versioning.ExactVersion', 'Multipart.Automatic', 'Multipart.ListParts',
    'Multipart.Abort', 'MultipartUploads.List', 'ObjectVersions.List',
    'Object.Delete', 'Bucket.Delete'
)

function Initialize-ClientVersions {
    param([hashtable]$Clients)
    foreach ($name in @('aws', 'mcli', 'rclone')) {
        $client = $Clients[$name]
        if ($null -eq $client) {
            continue
        }
        $versionArguments = switch ($name) {
            'aws' { @('--version') }
            'mcli' { @('--version') }
            'rclone' { @('version') }
        }
        $versionResult = Invoke-NativeCommand -Command $client.Path -Arguments $versionArguments -AllowFailure
        $version = ($versionResult.Output -split "`r?`n" | Select-Object -First 1).Trim()
        if ([string]::IsNullOrWhiteSpace($version)) {
            $version = 'detected; version unavailable'
        }
        $client | Add-Member -NotePropertyName Version -NotePropertyValue $version -Force
        Add-CompatResult -Client $name -Operation Client.Availability -Status PASS -Message $version
    }
}

function Add-MissingClientResults {
    param([hashtable]$Clients)
    $definitions = @{
        aws = $script:AwsOperations
        mcli = $script:McliOperations
        rclone = $script:RcloneOperations
    }
    foreach ($name in @('aws', 'mcli', 'rclone')) {
        if ($null -ne $Clients[$name]) {
            continue
        }
        Add-SkipResult -Client $name -Operation Client.Availability -Reason "$name executable was not found in PATH; the script does not install clients"
        foreach ($operation in $definitions[$name]) {
            Add-SkipResult -Client $name -Operation $operation -Reason "$name is not installed"
        }
    }
}

function Invoke-DetectedClientMatrices {
    param(
        [hashtable]$Clients,
        [string]$Endpoint,
        [string]$Region,
        [object]$Credential,
        [string]$TempRoot,
        [string]$RunId
    )
    Add-MissingClientResults -Clients $Clients
    if ($null -ne $Clients.aws) {
        Invoke-AwsCompatibilityMatrix -Client $Clients.aws -Endpoint $Endpoint -Region $Region -Credential $Credential -TempRoot $TempRoot -RunId $RunId
    }
    if ($null -ne $Clients.mcli) {
        Invoke-McliCompatibilityMatrix -Client $Clients.mcli -Endpoint $Endpoint -Credential $Credential -TempRoot $TempRoot -RunId $RunId
    }
    if ($null -ne $Clients.rclone) {
        Invoke-RcloneCompatibilityMatrix -Client $Clients.rclone -Endpoint $Endpoint -Region $Region -Credential $Credential -TempRoot $TempRoot -RunId $RunId
    }
}

function Invoke-AwsApi {
    param(
        [object]$Client,
        [string]$Endpoint,
        [string]$Region,
        [string]$Operation,
        [string[]]$Arguments = @(),
        [string[]]$GlobalArguments = @(),
        [switch]$AllowFailure
    )
    $allArguments = @(
        '--endpoint-url', $Endpoint,
        '--region', $Region,
        '--output', 'json'
    ) + $GlobalArguments + @('s3api', $Operation) + $Arguments
    return Invoke-NativeCommand -Command $Client.Path -Arguments $allArguments -AllowFailure:$AllowFailure
}

function Get-JsonPropertyString {
    param([object]$Object, [string]$Name)
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property -or $null -eq $property.Value) {
        return $null
    }
    return [string]$property.Value
}

function New-TestBytesFile {
    param([string]$Path, [int]$Bytes)
    $buffer = New-Object byte[] $Bytes
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $rng.GetBytes($buffer)
    }
    finally {
        $rng.Dispose()
    }
    [IO.File]::WriteAllBytes($Path, $buffer)
}

function Assert-AwsBucketDoesNotExist {
    param(
        [object]$Client,
        [string]$Endpoint,
        [string]$Region,
        [string]$Bucket
    )
    $probe = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-bucket -Arguments @('--bucket', $Bucket) -AllowFailure
    Assert-Condition ($probe.ExitCode -ne 0 -and $probe.Output -match '(?i)(404|NoSuchBucket|Not Found)') "refusing to use bucket '$Bucket' because the preflight did not prove it absent"
}

function ConvertFrom-AwsTimestamp {
    param([string]$Value, [string]$Context)
    try {
        return [DateTimeOffset]::Parse(
            $Value,
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::RoundtripKind
        )
    }
    catch {
        throw "${Context} returned an invalid timestamp: $Value"
    }
}

function Assert-AwsTagSet {
    param(
        [object]$Response,
        [hashtable]$Expected,
        [string]$Context
    )
    $tagSetProperty = $Response.PSObject.Properties['TagSet']
    Assert-Condition ($null -ne $tagSetProperty) "$Context did not return TagSet"
    $tags = @()
    if ($null -ne $tagSetProperty.Value) {
        $tags = @($tagSetProperty.Value)
    }
    Assert-Condition ($tags.Count -eq $Expected.Count) "$Context returned $($tags.Count) tags; expected $($Expected.Count)"

    $actual = @{}
    foreach ($tag in $tags) {
        $keyProperty = $tag.PSObject.Properties['Key']
        $valueProperty = $tag.PSObject.Properties['Value']
        Assert-Condition ($null -ne $keyProperty -and $null -ne $valueProperty) "$Context returned a tag without Key or Value"
        $key = [string]$keyProperty.Value
        Assert-Condition (-not $actual.ContainsKey($key)) "$Context returned duplicate tag key '$key'"
        $actual[$key] = [string]$valueProperty.Value
    }
    foreach ($key in $Expected.Keys) {
        Assert-Condition ($actual.ContainsKey([string]$key)) "$Context omitted tag key '$key'"
        Assert-Condition ([string]$actual[[string]$key] -eq [string]$Expected[$key]) "$Context returned the wrong value for tag key '$key'"
    }
}

function Add-AwsTrackedVersion {
    param(
        [System.Collections.Generic.List[object]]$Versions,
        [string]$Key,
        [string]$VersionId,
        [string]$Context
    )
    Assert-Condition (-not [string]::IsNullOrWhiteSpace($VersionId) -and $VersionId -ne 'null') "$Context did not return an opaque VersionId"
    $Versions.Add([pscustomobject]@{ Key = $Key; VersionId = $VersionId })
}

function Invoke-AwsTrackedVersionCleanup {
    param(
        [object]$Client,
        [string]$Endpoint,
        [string]$Region,
        [string]$Bucket,
        [System.Collections.Generic.List[object]]$Versions
    )
    $failures = New-Object System.Collections.Generic.List[string]
    foreach ($version in @($Versions)) {
        $key = [string]$version.Key
        $versionId = [string]$version.VersionId
        if ([string]::IsNullOrWhiteSpace($key) -or [string]::IsNullOrWhiteSpace($versionId) -or $versionId -eq 'null') {
            $failures.Add('tracked Tagging cleanup entry did not contain an exact key and opaque VersionId')
            continue
        }
        $deleted = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $Bucket, '--key', $key, '--version-id', $versionId) -AllowFailure
        if ($deleted.ExitCode -ne 0 -and $deleted.Output -notmatch '(?i)(404|NoSuchKey|NoSuchVersion|Not Found)') {
            $failures.Add(("could not delete tracked Tagging version {0} / {1}: {2}" -f $key, $versionId, $deleted.Output))
            continue
        }
        $probe = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-object -Arguments @('--bucket', $Bucket, '--key', $key, '--version-id', $versionId) -AllowFailure
        if ($probe.ExitCode -eq 0 -or $probe.Output -notmatch '(?i)(404|NoSuchKey|NoSuchVersion|Not Found)') {
            $failures.Add("tracked Tagging version still exists after cleanup: $key / $versionId")
        }
    }
    if ($failures.Count -gt 0) {
        return [pscustomobject]@{ Success = $false; Message = ($failures -join '; ') }
    }
    return [pscustomobject]@{ Success = $true; Message = "deleted and verified $($Versions.Count) exact Tagging object versions" }
}

function Invoke-AwsTagCountCase {
    param(
        [object]$Client,
        [string]$Endpoint,
        [string]$Region,
        [string]$Bucket,
        [string]$Key,
        [string]$VersionId,
        [int]$ExpectedCount,
        [string]$DownloadPath
    )
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        $head = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-object -Arguments @('--bucket', $Bucket, '--key', $Key, '--version-id', $VersionId)).Output -Context 'Tagging HeadObject'
        $get = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object -Arguments @('--bucket', $Bucket, '--key', $Key, '--version-id', $VersionId, $DownloadPath)).Output -Context 'Tagging GetObject'
        $observed = New-Object System.Collections.Generic.List[string]
        foreach ($response in @(@{ Name = 'HeadObject'; Value = $head }, @{ Name = 'GetObject'; Value = $get })) {
            $property = $response.Value.PSObject.Properties['TagCount']
            if ($null -ne $property) {
                Assert-Condition ([int]$property.Value -eq $ExpectedCount) "$($response.Name) TagCount was $($property.Value); expected $ExpectedCount"
                $observed.Add([string]$response.Name)
            }
        }
        $watch.Stop()
        if ($observed.Count -eq 0) {
            Add-CompatResult -Client aws -Operation Tagging.TagCount -Status SKIP -DurationMs $watch.ElapsedMilliseconds -Message 'installed AWS CLI response model exposed TagCount on neither HeadObject nor GetObject'
        }
        else {
            Add-CompatResult -Client aws -Operation Tagging.TagCount -Status PASS -DurationMs $watch.ElapsedMilliseconds -Message ("TagCount=$ExpectedCount matched on {0}" -f ($observed -join ' and '))
        }
    }
    catch {
        $watch.Stop()
        Add-CompatResult -Client aws -Operation Tagging.TagCount -Status FAIL -DurationMs $watch.ElapsedMilliseconds -Message $_.Exception.Message
    }
}

function Invoke-AwsObjectLockBucketCleanup {
    param(
        [object]$Client,
        [string]$Endpoint,
        [string]$Region,
        [string]$Bucket
    )
    $failures = New-Object System.Collections.Generic.List[string]
    $listed = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation list-object-versions -Arguments @('--bucket', $Bucket) -AllowFailure
    if ($listed.ExitCode -ne 0) {
        if ($listed.Output -match '(?i)(404|NoSuchBucket|Not Found)') {
            return [pscustomobject]@{ Success = $true; Message = 'Object Lock bucket was already absent' }
        }
        $failures.Add("could not list exact versions before cleanup: $($listed.Output)")
    }
    else {
        try {
            $state = ConvertFrom-CompatJson -Json $listed.Output -Context 'Object Lock cleanup ListObjectVersions'
            $versionsProperty = $state.PSObject.Properties['Versions']
            $versions = if ($null -eq $versionsProperty) { @() } else { @($versionsProperty.Value) }
            foreach ($version in $versions) {
                $key = [string]$version.Key
                $versionId = [string]$version.VersionId
                if ([string]::IsNullOrWhiteSpace($key) -or [string]::IsNullOrWhiteSpace($versionId)) {
                    $failures.Add('ListObjectVersions returned a version without Key or VersionId during cleanup')
                    continue
                }
                $legalHoldOff = @{ Status = 'OFF' } | ConvertTo-Json -Compress
                $hold = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation put-object-legal-hold -Arguments @('--bucket', $Bucket, '--key', $key, '--version-id', $versionId, '--legal-hold', $legalHoldOff) -AllowFailure
                if ($hold.ExitCode -ne 0 -and $hold.Output -notmatch '(?i)(404|NoSuchKey|NoSuchVersion|Not Found)') {
                    $failures.Add("could not set legal hold OFF for $key version $versionId before cleanup: $($hold.Output)")
                    continue
                }
                $deleted = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $Bucket, '--key', $key, '--version-id', $versionId, '--bypass-governance-retention') -AllowFailure
                if ($deleted.ExitCode -ne 0 -and $deleted.Output -notmatch '(?i)(404|NoSuchKey|NoSuchVersion|Not Found)') {
                    $failures.Add("could not bypass governance retention for $key version $versionId during cleanup: $($deleted.Output)")
                }
            }
            $markersProperty = $state.PSObject.Properties['DeleteMarkers']
            $markers = if ($null -eq $markersProperty) { @() } else { @($markersProperty.Value) }
            foreach ($marker in $markers) {
                $key = [string]$marker.Key
                $versionId = [string]$marker.VersionId
                if ([string]::IsNullOrWhiteSpace($key) -or [string]::IsNullOrWhiteSpace($versionId)) {
                    $failures.Add('ListObjectVersions returned a delete marker without Key or VersionId during cleanup')
                    continue
                }
                $deleted = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $Bucket, '--key', $key, '--version-id', $versionId) -AllowFailure
                if ($deleted.ExitCode -ne 0 -and $deleted.Output -notmatch '(?i)(404|NoSuchKey|NoSuchVersion|Not Found)') {
                    $failures.Add("could not delete marker $key version $versionId during cleanup: $($deleted.Output)")
                }
            }
        }
        catch {
            $failures.Add($_.Exception.Message)
        }
    }

    $deletedBucket = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-bucket -Arguments @('--bucket', $Bucket) -AllowFailure
    if ($deletedBucket.ExitCode -ne 0 -and $deletedBucket.Output -notmatch '(?i)(404|NoSuchBucket|Not Found)') {
        $failures.Add("could not delete Object Lock bucket: $($deletedBucket.Output)")
    }
    $probe = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-bucket -Arguments @('--bucket', $Bucket) -AllowFailure
    if ($probe.ExitCode -eq 0 -or $probe.Output -notmatch '(?i)(404|NoSuchBucket|Not Found)') {
        $failures.Add("Object Lock bucket still exists after cleanup: $($probe.Output)")
    }
    if ($failures.Count -gt 0) {
        return [pscustomobject]@{ Success = $false; Message = ($failures -join '; ') }
    }
    return [pscustomobject]@{ Success = $true; Message = 'legal hold released, exact versions governance-bypassed, and Object Lock bucket deleted' }
}

function Invoke-AwsCompatibilityMatrix {
    param(
        [object]$Client,
        [string]$Endpoint,
        [string]$Region,
        [object]$Credential,
        [string]$TempRoot,
        [string]$RunId
    )
    Write-CompatInfo 'running AWS CLI matrix'
    Set-ScopedEnvironmentVariable AWS_ACCESS_KEY_ID $Credential.AccessKeyId
    Set-ScopedEnvironmentVariable AWS_SECRET_ACCESS_KEY $Credential.SecretAccessKey
    Set-ScopedEnvironmentVariable AWS_DEFAULT_REGION $Region
    Set-ScopedEnvironmentVariable AWS_EC2_METADATA_DISABLED 'true'
    Set-ScopedEnvironmentVariable AWS_PAGER ''
    Set-ScopedEnvironmentVariable AWS_CLI_AUTO_PROMPT 'off'

    $work = Join-Path $TempRoot 'aws'
    New-Item -ItemType Directory -Path $work -Force | Out-Null
    $bucket = "prismark-compat-aws-$RunId"
    $lockBucket = "prismark-compat-aws-lock-$RunId"
    $payloadPath = Join-Path $work 'payload.txt'
    $downloadPath = Join-Path $work 'download.txt'
    $rangePath = Join-Path $work 'range.txt'
    $conditionalPath = Join-Path $work 'conditional.txt'
    $payload = "PrismArk-compatible-payload-$RunId"
    [IO.File]::WriteAllText($payloadPath, $payload, (New-Object Text.UTF8Encoding($false)))
    $context = @{
        ETag = $null
        NullKey = 'versions/item.txt'
        V1 = $null
        V2 = $null
        DeleteMarker = $null
        MultipartVersion = $null
        MultipartUploads = New-Object System.Collections.Generic.List[object]
        BucketCreated = $false
        LockBucketCreated = $false
        LockKey = 'object-lock/explicit.txt'
        LockVersion = $null
        LockRetainUntil = $null
        TagKey = 'tagging/source.txt'
        TagCopyDefaultKey = 'tagging/copy-default.txt'
        TagCopyReplaceKey = 'tagging/copy-replace.txt'
        TagV1 = $null
        TagV2 = $null
        TagVersions = New-Object System.Collections.Generic.List[object]
    }
    $multipartKey = 'multipart/complete.bin'
    $knownNullKeys = @(
        'basic.txt', 'dir/a.txt', 'dir/sub/b.txt', 'other.txt',
        'page/1.txt', 'page/2.txt', 'page/3.txt', 'batch/1.txt', 'batch/2.txt',
        'copy/probe.txt', 'copy/upload-part-copy.txt', 'multipart/list-probe.bin',
        $context.NullKey, $multipartKey, $context.TagKey,
        $context.TagCopyDefaultKey, $context.TagCopyReplaceKey
    )

    try {
    Invoke-CompatCase -Client aws -Operation Bucket.Create -Body {
        Assert-AwsBucketDoesNotExist -Client $Client -Endpoint $Endpoint -Region $Region -Bucket $bucket
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation create-bucket -Arguments @('--bucket', $bucket) | Out-Null
        $context.BucketCreated = $true
        'bucket created'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Bucket.Head -Body {
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-bucket -Arguments @('--bucket', $bucket) | Out-Null
        'HEAD bucket succeeded'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Bucket.List -Body {
        $result = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation list-buckets
        $json = ConvertFrom-CompatJson -Json $result.Output -Context 'list-buckets'
        $names = @($json.Buckets | ForEach-Object { $_.Name })
        Assert-Condition ($names -contains $bucket) 'created bucket was absent from ListBuckets'
        'ListBuckets contains the created bucket'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Object.Put -Body {
        $result = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation put-object -Arguments @('--bucket', $bucket, '--key', 'basic.txt', '--body', $payloadPath, '--content-type', 'text/plain', '--metadata', 'compat=aws')
        $json = ConvertFrom-CompatJson -Json $result.Output -Context 'put-object'
        $context.ETag = Get-JsonPropertyString -Object $json -Name ETag
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.ETag)) 'PutObject did not return an ETag'
        'PutObject returned an ETag'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Object.Head -Body {
        $result = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-object -Arguments @('--bucket', $bucket, '--key', 'basic.txt')
        $json = ConvertFrom-CompatJson -Json $result.Output -Context 'head-object'
        Assert-Condition ([long]$json.ContentLength -eq ([IO.FileInfo]$payloadPath).Length) 'HeadObject ContentLength mismatch'
        Assert-Condition ([string]$json.ContentType -eq 'text/plain') 'HeadObject ContentType mismatch'
        Assert-Condition ([string]$json.Metadata.compat -eq 'aws') 'HeadObject user metadata mismatch'
        'size, content type, and user metadata match'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Object.Get -Body {
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object -Arguments @('--bucket', $bucket, '--key', 'basic.txt', $downloadPath) | Out-Null
        Assert-Condition ((Get-FileSha256 $payloadPath) -eq (Get-FileSha256 $downloadPath)) 'GetObject body checksum mismatch'
        'downloaded body checksum matches'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Object.Range -Body {
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object -Arguments @('--bucket', $bucket, '--key', 'basic.txt', '--range', 'bytes=2-7', $rangePath) | Out-Null
        $source = [IO.File]::ReadAllBytes($payloadPath)
        $expected = New-Object byte[] 6
        [Array]::Copy($source, 2, $expected, 0, 6)
        Assert-Condition ([Convert]::ToBase64String($expected) -eq [Convert]::ToBase64String([IO.File]::ReadAllBytes($rangePath))) 'Range body mismatch'
        'single byte range matches'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Object.IfMatch -Body {
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object -Arguments @('--bucket', $bucket, '--key', 'basic.txt', '--if-match', $context.ETag, $conditionalPath) | Out-Null
        Assert-Condition ((Get-FileSha256 $payloadPath) -eq (Get-FileSha256 $conditionalPath)) 'If-Match body checksum mismatch'
        'matching ETag returns the object'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Object.IfNoneMatch -Body {
        $result = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object -Arguments @('--bucket', $bucket, '--key', 'basic.txt', '--if-none-match', $context.ETag, (Join-Path $work 'not-modified.txt')) -AllowFailure
        Assert-Condition ($result.ExitCode -ne 0) 'If-None-Match unexpectedly returned the object'
        Assert-Condition ($result.Output -match '(?i)(304|Not Modified)') "If-None-Match failed without a 304/Not Modified diagnostic: $($result.Output)"
        'matching ETag returns Not Modified'
    } | Out-Null

    foreach ($key in @('dir/a.txt', 'dir/sub/b.txt', 'other.txt', 'page/1.txt', 'page/2.txt', 'page/3.txt', 'batch/1.txt', 'batch/2.txt')) {
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation put-object -Arguments @('--bucket', $bucket, '--key', $key, '--body', $payloadPath) | Out-Null
    }
    Invoke-CompatCase -Client aws -Operation ListObjectsV2.PrefixDelimiter -Body {
        $result = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation list-objects-v2 -Arguments @('--bucket', $bucket, '--prefix', 'dir/', '--delimiter', '/')
        $json = ConvertFrom-CompatJson -Json $result.Output -Context 'list-objects-v2 prefix/delimiter'
        $keys = @($json.Contents | ForEach-Object { $_.Key })
        $prefixes = @($json.CommonPrefixes | ForEach-Object { $_.Prefix })
        Assert-Condition ($keys -contains 'dir/a.txt') 'prefix listing omitted dir/a.txt'
        Assert-Condition ($prefixes -contains 'dir/sub/') 'delimiter listing omitted dir/sub/'
        'prefix and common prefix are correct'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation ListObjectsV2.Pagination -Body {
        $first = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation list-objects-v2 -Arguments @('--bucket', $bucket, '--prefix', 'page/', '--max-keys', '1')).Output -Context 'first paginated list'
        $token = Get-JsonPropertyString -Object $first -Name NextContinuationToken
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($token)) 'first page did not return NextContinuationToken'
        $second = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation list-objects-v2 -Arguments @('--bucket', $bucket, '--prefix', 'page/', '--max-keys', '1', '--continuation-token', $token)).Output -Context 'second paginated list'
        Assert-Condition (@($first.Contents).Count -eq 1 -and @($second.Contents).Count -eq 1) 'pagination did not return one object per page'
        Assert-Condition ([string]$first.Contents[0].Key -ne [string]$second.Contents[0].Key) 'continuation token repeated the first object'
        'continuation token advances to the next object'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation DeleteObjects -Body {
        $deleteJson = @{ Objects = @(@{ Key = 'batch/1.txt' }, @{ Key = 'batch/2.txt' }); Quiet = $false } | ConvertTo-Json -Depth 5 -Compress
        $result = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-objects -Arguments @('--bucket', $bucket, '--delete', $deleteJson)
        $json = ConvertFrom-CompatJson -Json $result.Output -Context 'delete-objects'
        Assert-Condition (@($json.Deleted).Count -eq 2) 'DeleteObjects did not report two deleted objects'
        'two objects deleted in one request'
    } | Out-Null

    $copyProbeKey = 'copy/probe.txt'
    Invoke-CompatCase -Client aws -Operation CopyObject -Body {
        $copy = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation copy-object -Arguments @('--bucket', $bucket, '--key', $copyProbeKey, '--copy-source', "$bucket/basic.txt")
        $json = ConvertFrom-CompatJson -Json $copy.Output -Context 'copy-object'
        Assert-Condition (-not [string]::IsNullOrWhiteSpace([string]$json.CopyObjectResult.ETag)) 'CopyObject returned success without CopyObjectResult.ETag'
        $head = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-object -Arguments @('--bucket', $bucket, '--key', $copyProbeKey)).Output -Context 'head copied object'
        Assert-Condition ([long]$head.ContentLength -eq ([IO.FileInfo]$payloadPath).Length) 'CopyObject returned success but the copied object size is wrong'
    } | Out-Null
    Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $bucket, '--key', $copyProbeKey) -AllowFailure | Out-Null

    $partCopyKey = 'copy/upload-part-copy.txt'
    Invoke-CompatCase -Client aws -Operation UploadPartCopy -Body {
        $probeUploadId = $null
        try {
            $created = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation create-multipart-upload -Arguments @('--bucket', $bucket, '--key', $partCopyKey)).Output -Context 'create UploadPartCopy probe'
            $probeUploadId = [string]$created.UploadId
            Assert-Condition (-not [string]::IsNullOrWhiteSpace($probeUploadId)) 'UploadPartCopy probe did not receive an UploadId'
            $copied = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation upload-part-copy -Arguments @('--bucket', $bucket, '--key', $partCopyKey, '--upload-id', $probeUploadId, '--part-number', '1', '--copy-source', "$bucket/basic.txt")
            $copyJson = ConvertFrom-CompatJson -Json $copied.Output -Context 'upload-part-copy'
            $copyEtag = [string]$copyJson.CopyPartResult.ETag
            Assert-Condition (-not [string]::IsNullOrWhiteSpace($copyEtag)) 'UploadPartCopy returned success without CopyPartResult.ETag'
            $manifest = @{ Parts = @(@{ ETag = $copyEtag; PartNumber = 1 }) } | ConvertTo-Json -Depth 5 -Compress
            Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation complete-multipart-upload -Arguments @('--bucket', $bucket, '--key', $partCopyKey, '--upload-id', $probeUploadId, '--multipart-upload', $manifest) | Out-Null
            $copiedPath = Join-Path $work 'upload-part-copy.txt'
            Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object -Arguments @('--bucket', $bucket, '--key', $partCopyKey, $copiedPath) | Out-Null
            Assert-Condition ((Get-FileSha256 $payloadPath) -eq (Get-FileSha256 $copiedPath)) 'UploadPartCopy returned success but completed body checksum differs from the source'
            $probeUploadId = $null
        }
        finally {
            if (-not [string]::IsNullOrWhiteSpace([string]$probeUploadId)) {
                Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation abort-multipart-upload -Arguments @('--bucket', $bucket, '--key', $partCopyKey, '--upload-id', $probeUploadId) -AllowFailure | Out-Null
            }
            Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $bucket, '--key', $partCopyKey) -AllowFailure | Out-Null
        }
    } | Out-Null

    Invoke-CompatCase -Client aws -Operation MultipartUploads.List -Body {
        $listProbeKey = 'multipart/list-probe.bin'
        $listProbeId = $null
        try {
            $created = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation create-multipart-upload -Arguments @('--bucket', $bucket, '--key', $listProbeKey)).Output -Context 'create ListMultipartUploads probe'
            $listProbeId = [string]$created.UploadId
            Assert-Condition (-not [string]::IsNullOrWhiteSpace($listProbeId)) 'ListMultipartUploads probe did not receive an UploadId'
            $listed = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation list-multipart-uploads -Arguments @('--bucket', $bucket)
            $listJson = ConvertFrom-CompatJson -Json $listed.Output -Context 'list-multipart-uploads'
            $uploadIds = @($listJson.Uploads | ForEach-Object { [string]$_.UploadId })
            Assert-Condition ($uploadIds -contains $listProbeId) 'ListMultipartUploads returned success but omitted the known pending UploadId'
        }
        finally {
            if (-not [string]::IsNullOrWhiteSpace([string]$listProbeId)) {
                Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation abort-multipart-upload -Arguments @('--bucket', $bucket, '--key', $listProbeKey, '--upload-id', $listProbeId) -AllowFailure | Out-Null
            }
        }
    } | Out-Null

    $preVersioningKeys = @('basic.txt', 'dir/a.txt', 'dir/sub/b.txt', 'other.txt', 'page/1.txt', 'page/2.txt', 'page/3.txt')
    $preDeleteJson = @{ Objects = @($preVersioningKeys | ForEach-Object { @{ Key = $_ } }); Quiet = $true } | ConvertTo-Json -Depth 5 -Compress
    Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-objects -Arguments @('--bucket', $bucket, '--delete', $preDeleteJson) | Out-Null

    Invoke-CompatCase -Client aws -Operation Versioning.NullVersion -Body {
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation put-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey, '--body', $payloadPath) | Out-Null
        $head = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey, '--version-id', 'null')).Output -Context 'head null version'
        Assert-Condition ([string]$head.VersionId -eq 'null') 'explicit null version did not return VersionId=null'
        'unversioned object is addressable as versionId=null'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Versioning.Enable -Body {
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation put-bucket-versioning -Arguments @('--bucket', $bucket, '--versioning-configuration', 'Status=Enabled') | Out-Null
        $state = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-bucket-versioning -Arguments @('--bucket', $bucket)).Output -Context 'get-bucket-versioning'
        Assert-Condition ([string]$state.Status -eq 'Enabled') 'bucket versioning did not become Enabled'
        'bucket versioning enabled'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Tagging.PutObject -Body {
        $put = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation put-object -Arguments @(
            '--bucket', $bucket,
            '--key', $context.TagKey,
            '--body', $payloadPath,
            '--tagging', 'project=prismark&variant=image%2Fwebp'
        )).Output -Context 'tagged put-object'
        $context.TagV1 = Get-JsonPropertyString -Object $put -Name VersionId
        Add-AwsTrackedVersion -Versions $context.TagVersions -Key $context.TagKey -VersionId $context.TagV1 -Context 'tagged PutObject'
        $readBack = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object-tagging -Arguments @('--bucket', $bucket, '--key', $context.TagKey, '--version-id', $context.TagV1)).Output -Context 'get initial object tagging'
        Assert-Condition ([string]$readBack.VersionId -eq $context.TagV1) 'GetObjectTagging did not report the exact tagged PutObject VersionId'
        Assert-AwsTagSet -Response $readBack -Expected @{ project = 'prismark'; variant = 'image/webp' } -Context 'initial GetObjectTagging'
        'PutObject tags were decoded and read back from the exact VersionId'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Tagging.PutObjectTagging -Body {
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.TagV1)) 'initial tagged VersionId is unavailable'
        $replacement = @{
            TagSet = @(
                @{ Key = 'stage'; Value = 'review' },
                @{ Key = 'owner'; Value = 'prismark' }
            )
        } | ConvertTo-Json -Depth 5 -Compress
        $put = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation put-object-tagging -Arguments @('--bucket', $bucket, '--key', $context.TagKey, '--version-id', $context.TagV1, '--tagging', $replacement)).Output -Context 'put-object-tagging replacement'
        Assert-Condition ([string]$put.VersionId -eq $context.TagV1) 'PutObjectTagging did not report the exact target VersionId'
        $readBack = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object-tagging -Arguments @('--bucket', $bucket, '--key', $context.TagKey, '--version-id', $context.TagV1)).Output -Context 'get replaced object tagging'
        Assert-AwsTagSet -Response $readBack -Expected @{ stage = 'review'; owner = 'prismark' } -Context 'replaced GetObjectTagging'
        'PutObjectTagging replaced, rather than merged, the exact version tag set'
    } | Out-Null
    Invoke-AwsTagCountCase -Client $Client -Endpoint $Endpoint -Region $Region -Bucket $bucket -Key $context.TagKey -VersionId $context.TagV1 -ExpectedCount 2 -DownloadPath (Join-Path $work 'tag-count-get.txt')
    Invoke-CompatCase -Client aws -Operation Tagging.VersionIsolation -Body {
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.TagV1)) 'initial tagged VersionId is unavailable'
        $put = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation put-object -Arguments @(
            '--bucket', $bucket,
            '--key', $context.TagKey,
            '--body', $payloadPath,
            '--tagging', 'generation=two&scope=latest'
        )).Output -Context 'second tagged put-object'
        $context.TagV2 = Get-JsonPropertyString -Object $put -Name VersionId
        Add-AwsTrackedVersion -Versions $context.TagVersions -Key $context.TagKey -VersionId $context.TagV2 -Context 'second tagged PutObject'
        Assert-Condition ($context.TagV2 -ne $context.TagV1) 'second tagged PutObject reused the first VersionId'
        $first = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object-tagging -Arguments @('--bucket', $bucket, '--key', $context.TagKey, '--version-id', $context.TagV1)).Output -Context 'get first isolated tag set'
        $second = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object-tagging -Arguments @('--bucket', $bucket, '--key', $context.TagKey, '--version-id', $context.TagV2)).Output -Context 'get second isolated tag set'
        Assert-AwsTagSet -Response $first -Expected @{ stage = 'review'; owner = 'prismark' } -Context 'first isolated version'
        Assert-AwsTagSet -Response $second -Expected @{ generation = 'two'; scope = 'latest' } -Context 'second isolated version'
        'two versions of the same key retain independent tag sets'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Tagging.CopyDefault -Body {
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.TagV1)) 'source tagged VersionId is unavailable'
        $copySource = "${bucket}/$($context.TagKey)?versionId=$([Uri]::EscapeDataString($context.TagV1))"
        $copied = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation copy-object -Arguments @('--bucket', $bucket, '--key', $context.TagCopyDefaultKey, '--copy-source', $copySource)).Output -Context 'default tagging CopyObject'
        $copyVersion = Get-JsonPropertyString -Object $copied -Name VersionId
        Add-AwsTrackedVersion -Versions $context.TagVersions -Key $context.TagCopyDefaultKey -VersionId $copyVersion -Context 'default tagging CopyObject'
        Assert-Condition (-not [string]::IsNullOrWhiteSpace([string]$copied.CopyObjectResult.ETag)) 'default tagging CopyObject returned no ETag'
        $readBack = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object-tagging -Arguments @('--bucket', $bucket, '--key', $context.TagCopyDefaultKey, '--version-id', $copyVersion)).Output -Context 'get default copied tags'
        Assert-AwsTagSet -Response $readBack -Expected @{ stage = 'review'; owner = 'prismark' } -Context 'default COPY tag set'
        'CopyObject default COPY preserved the exact source version tag set'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Tagging.CopyReplace -Body {
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.TagV2)) 'second tagged VersionId is unavailable'
        $copySource = "${bucket}/$($context.TagKey)?versionId=$([Uri]::EscapeDataString($context.TagV2))"
        $copied = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation copy-object -Arguments @(
            '--bucket', $bucket,
            '--key', $context.TagCopyReplaceKey,
            '--copy-source', $copySource,
            '--tagging-directive', 'REPLACE',
            '--tagging', 'copy=replace&variant=thumbnail'
        )).Output -Context 'replacement tagging CopyObject'
        $copyVersion = Get-JsonPropertyString -Object $copied -Name VersionId
        Add-AwsTrackedVersion -Versions $context.TagVersions -Key $context.TagCopyReplaceKey -VersionId $copyVersion -Context 'replacement tagging CopyObject'
        Assert-Condition (-not [string]::IsNullOrWhiteSpace([string]$copied.CopyObjectResult.ETag)) 'replacement tagging CopyObject returned no ETag'
        $readBack = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object-tagging -Arguments @('--bucket', $bucket, '--key', $context.TagCopyReplaceKey, '--version-id', $copyVersion)).Output -Context 'get replacement copied tags'
        Assert-AwsTagSet -Response $readBack -Expected @{ copy = 'replace'; variant = 'thumbnail' } -Context 'REPLACE COPY tag set'
        'CopyObject REPLACE stored only the x-amz-tagging replacement set'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Tagging.DeleteObjectTagging -Body {
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.TagV1) -and -not [string]::IsNullOrWhiteSpace($context.TagV2)) 'tagged VersionIds are unavailable'
        $deleted = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object-tagging -Arguments @('--bucket', $bucket, '--key', $context.TagKey, '--version-id', $context.TagV1)).Output -Context 'delete-object-tagging'
        Assert-Condition ([string]$deleted.VersionId -eq $context.TagV1) 'DeleteObjectTagging did not report the exact target VersionId'
        $empty = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object-tagging -Arguments @('--bucket', $bucket, '--key', $context.TagKey, '--version-id', $context.TagV1)).Output -Context 'get deleted object tagging'
        $untouched = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object-tagging -Arguments @('--bucket', $bucket, '--key', $context.TagKey, '--version-id', $context.TagV2)).Output -Context 'get untouched second version tagging'
        Assert-AwsTagSet -Response $empty -Expected @{} -Context 'deleted exact-version tag set'
        Assert-AwsTagSet -Response $untouched -Expected @{ generation = 'two'; scope = 'latest' } -Context 'untouched second-version tag set'
        'DeleteObjectTagging emptied only the exact target version tag set'
    } | Out-Null
    Add-SkipResult -Client aws -Operation Tagging.InvalidTag.Negative -Reason 'AWS CLI may reject invalid tag shapes before transmission, and this matrix has no controlled raw SigV4 HTTP helper; no server-side result is claimed'
    Add-SkipResult -Client aws -Operation Tagging.DuplicateKey.Negative -Reason 'AWS CLI may validate duplicate tag keys locally, and this matrix has no controlled raw SigV4 HTTP helper; no server-side result is claimed'
    Add-SkipResult -Client aws -Operation Tagging.TooMany.Negative -Reason 'AWS CLI may enforce the ten-tag model before transmission, and this matrix has no controlled raw SigV4 HTTP helper; no server-side result is claimed'
    Add-SkipResult -Client aws -Operation Tagging.BadPercentEncoding.Negative -Reason 'AWS CLI does not provide a reliable raw x-amz-tagging header surface for malformed percent encoding, and this matrix has no controlled raw SigV4 HTTP helper'
    Invoke-CompatCase -Client aws -Operation Versioning.ExactRead -Body {
        $v1 = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation put-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey, '--body', $payloadPath, '--metadata', 'generation=one')).Output -Context 'put version one'
        $v2Payload = Join-Path $work 'v2.txt'
        [IO.File]::WriteAllText($v2Payload, "version-two-$RunId", (New-Object Text.UTF8Encoding($false)))
        $v2 = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation put-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey, '--body', $v2Payload, '--metadata', 'generation=two')).Output -Context 'put version two'
        $context.V1 = Get-JsonPropertyString -Object $v1 -Name VersionId
        $context.V2 = Get-JsonPropertyString -Object $v2 -Name VersionId
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.V1) -and $context.V1 -ne 'null') 'first enabled PutObject did not return an opaque VersionId'
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.V2) -and $context.V2 -ne $context.V1) 'second enabled PutObject did not return a distinct VersionId'
        $exactPath = Join-Path $work 'exact-v1.txt'
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey, '--version-id', $context.V1, $exactPath) | Out-Null
        Assert-Condition ((Get-FileSha256 $payloadPath) -eq (Get-FileSha256 $exactPath)) 'exact version body mismatch'
        'opaque versions are distinct and exact version read succeeds'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Versioning.DeleteMarker -Body {
        $deleted = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey)).Output -Context 'delete marker creation'
        $context.DeleteMarker = Get-JsonPropertyString -Object $deleted -Name VersionId
        Assert-Condition ([bool]$deleted.DeleteMarker) 'DeleteObject did not report DeleteMarker=true'
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.DeleteMarker)) 'DeleteObject did not return delete marker VersionId'
        $head = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey) -AllowFailure
        Assert-Condition ($head.ExitCode -ne 0 -and $head.Output -match '(?i)(404|Not Found|NoSuchKey)') 'current HeadObject was not hidden by the delete marker'
        'delete marker hides the current object'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Versioning.ExactDelete -Body {
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey, '--version-id', $context.V1) | Out-Null
        $missing = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey, '--version-id', $context.V1) -AllowFailure
        Assert-Condition ($missing.ExitCode -ne 0 -and $missing.Output -match '(?i)(404|NoSuchVersion|Not Found)') 'deleted exact version remained readable'
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey, '--version-id', $context.DeleteMarker) | Out-Null
        $revealed = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey)).Output -Context 'head after delete-marker removal'
        Assert-Condition ([string]$revealed.VersionId -eq $context.V2) 'removing the delete marker did not reveal the latest data version'
        $context.V1 = $null
        $context.DeleteMarker = $null
        'exact data-version and delete-marker deletion behave correctly'
    } | Out-Null

    $part1 = Join-Path $work 'part1.bin'
    $part2 = Join-Path $work 'part2.bin'
    New-TestBytesFile -Path $part1 -Bytes (5 * 1024 * 1024)
    New-TestBytesFile -Path $part2 -Bytes 65537
    Invoke-CompatCase -Client aws -Operation Multipart.Create -Body {
        $created = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation create-multipart-upload -Arguments @('--bucket', $bucket, '--key', $multipartKey, '--content-type', 'application/octet-stream', '--metadata', 'compat=multipart')).Output -Context 'create-multipart-upload'
        Assert-Condition (-not [string]::IsNullOrWhiteSpace([string]$created.UploadId)) 'CreateMultipartUpload did not return UploadId'
        $context.MultipartUploads.Add([pscustomobject]@{ Key = $multipartKey; UploadId = [string]$created.UploadId })
        'multipart upload created'
    } | Out-Null
    $partEtags = @{}
    Invoke-CompatCase -Client aws -Operation Multipart.UploadPart -Body {
        $upload = $context.MultipartUploads[0]
        $one = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation upload-part -Arguments @('--bucket', $bucket, '--key', $upload.Key, '--upload-id', $upload.UploadId, '--part-number', '1', '--body', $part1)).Output -Context 'upload part one'
        $two = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation upload-part -Arguments @('--bucket', $bucket, '--key', $upload.Key, '--upload-id', $upload.UploadId, '--part-number', '2', '--body', $part2)).Output -Context 'upload part two'
        $partEtags[1] = [string]$one.ETag
        $partEtags[2] = [string]$two.ETag
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($partEtags[1]) -and -not [string]::IsNullOrWhiteSpace($partEtags[2])) 'UploadPart did not return both ETags'
        'two parts uploaded'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Multipart.ListParts -Body {
        $upload = $context.MultipartUploads[0]
        $listed = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation list-parts -Arguments @('--bucket', $bucket, '--key', $upload.Key, '--upload-id', $upload.UploadId)).Output -Context 'list-parts'
        Assert-Condition (@($listed.Parts).Count -eq 2) 'ListParts did not return two parts'
        'ListParts returned both uploaded parts'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Multipart.Complete -Body {
        $upload = $context.MultipartUploads[0]
        $manifest = @{ Parts = @(@{ ETag = $partEtags[1]; PartNumber = 1 }, @{ ETag = $partEtags[2]; PartNumber = 2 }) } | ConvertTo-Json -Depth 5 -Compress
        $completed = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation complete-multipart-upload -Arguments @('--bucket', $bucket, '--key', $upload.Key, '--upload-id', $upload.UploadId, '--multipart-upload', $manifest)).Output -Context 'complete-multipart-upload'
        $context.MultipartVersion = Get-JsonPropertyString -Object $completed -Name VersionId
        Assert-Condition (-not [string]::IsNullOrWhiteSpace([string]$completed.ETag) -and [string]$completed.ETag -match '-2"?$') 'CompleteMultipartUpload did not return a two-part multipart ETag'
        $context.MultipartUploads.RemoveAt(0)
        $combined = Join-Path $work 'combined.bin'
        $output = [IO.File]::Open($combined, [IO.FileMode]::Create, [IO.FileAccess]::Write)
        try {
            foreach ($sourcePath in @($part1, $part2)) {
                $input = [IO.File]::OpenRead($sourcePath)
                try { $input.CopyTo($output) } finally { $input.Dispose() }
            }
        }
        finally { $output.Dispose() }
        $download = Join-Path $work 'multipart-download.bin'
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object -Arguments @('--bucket', $bucket, '--key', $multipartKey, '--version-id', $context.MultipartVersion, $download) | Out-Null
        Assert-Condition ((Get-FileSha256 $combined) -eq (Get-FileSha256 $download)) 'completed multipart object checksum mismatch'
        'multipart completion returned the expected ETag and body'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Multipart.Abort -Body {
        $abortKey = 'multipart/abort.bin'
        $created = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation create-multipart-upload -Arguments @('--bucket', $bucket, '--key', $abortKey)).Output -Context 'create abort upload'
        $pending = [pscustomobject]@{ Key = $abortKey; UploadId = [string]$created.UploadId }
        $context.MultipartUploads.Add($pending)
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation upload-part -Arguments @('--bucket', $bucket, '--key', $abortKey, '--upload-id', $pending.UploadId, '--part-number', '1', '--body', $part2) | Out-Null
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation abort-multipart-upload -Arguments @('--bucket', $bucket, '--key', $abortKey, '--upload-id', $pending.UploadId) | Out-Null
        $context.MultipartUploads.Remove($pending) | Out-Null
        $listed = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation list-parts -Arguments @('--bucket', $bucket, '--key', $abortKey, '--upload-id', $pending.UploadId) -AllowFailure
        Assert-Condition ($listed.ExitCode -ne 0 -and $listed.Output -match '(?i)(404|NoSuchUpload|Not Found)') 'aborted multipart upload remained listable'
        'multipart upload aborted and is no longer listable'
    } | Out-Null

    Invoke-CompatCase -Client aws -Operation ObjectVersions.List -Body {
        $result = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation list-object-versions -Arguments @('--bucket', $bucket)
        $listed = ConvertFrom-CompatJson -Json $result.Output -Context 'list-object-versions'
        $versionIds = @($listed.Versions | ForEach-Object { [string]$_.VersionId })
        Assert-Condition ($versionIds -contains $context.V2) 'ListObjectVersions returned success but omitted a known live VersionId'
        Assert-Condition ($versionIds -contains $context.MultipartVersion) 'ListObjectVersions returned success but omitted the known multipart VersionId'
    } | Out-Null

    Invoke-CompatCase -Client aws -Operation ObjectLock.Bucket.Create -Body {
        Assert-AwsBucketDoesNotExist -Client $Client -Endpoint $Endpoint -Region $Region -Bucket $lockBucket
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation create-bucket -Arguments @('--bucket', $lockBucket, '--object-lock-enabled-for-bucket') | Out-Null
        $context.LockBucketCreated = $true
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-bucket -Arguments @('--bucket', $lockBucket) | Out-Null
        $versioning = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-bucket-versioning -Arguments @('--bucket', $lockBucket)).Output -Context 'Object Lock bucket versioning'
        Assert-Condition ([string]$versioning.Status -eq 'Enabled') 'Object Lock bucket did not enable versioning atomically'
        'new Object Lock bucket exists and reports versioning Enabled'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation ObjectLock.Bucket.GetConfiguration -Body {
        Assert-Condition $context.LockBucketCreated 'Object Lock bucket was not created by this run'
        $configuration = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object-lock-configuration -Arguments @('--bucket', $lockBucket)).Output -Context 'initial get-object-lock-configuration'
        Assert-Condition ([string]$configuration.ObjectLockConfiguration.ObjectLockEnabled -eq 'Enabled') 'new bucket ObjectLockEnabled was not Enabled'
        'GET Object Lock configuration reports Enabled'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation ObjectLock.Bucket.PutDefaultGovernance -Body {
        Assert-Condition $context.LockBucketCreated 'Object Lock bucket was not created by this run'
        $configuration = @{
            ObjectLockEnabled = 'Enabled'
            Rule = @{ DefaultRetention = @{ Mode = 'GOVERNANCE'; Days = 1 } }
        } | ConvertTo-Json -Depth 6 -Compress
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation put-object-lock-configuration -Arguments @('--bucket', $lockBucket, '--object-lock-configuration', $configuration) | Out-Null
        $readBack = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object-lock-configuration -Arguments @('--bucket', $lockBucket)).Output -Context 'read-back get-object-lock-configuration'
        $defaultRetention = $readBack.ObjectLockConfiguration.Rule.DefaultRetention
        Assert-Condition ([string]$readBack.ObjectLockConfiguration.ObjectLockEnabled -eq 'Enabled') 'PUT Object Lock configuration cleared ObjectLockEnabled'
        Assert-Condition ([string]$defaultRetention.Mode -eq 'GOVERNANCE') 'default retention mode was not GOVERNANCE'
        Assert-Condition ([int]$defaultRetention.Days -eq 1) 'default governance retention was not one day'
        'PUT configuration persisted default GOVERNANCE retention for one day'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation ObjectLock.Object.PutExplicit -Body {
        Assert-Condition $context.LockBucketCreated 'Object Lock bucket was not created by this run'
        $context.LockRetainUntil = [DateTimeOffset]::UtcNow.AddHours(2).ToString('yyyy-MM-ddTHH:mm:ssZ', [Globalization.CultureInfo]::InvariantCulture)
        $put = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation put-object -Arguments @(
            '--bucket', $lockBucket,
            '--key', $context.LockKey,
            '--body', $payloadPath,
            '--object-lock-mode', 'GOVERNANCE',
            '--object-lock-retain-until-date', $context.LockRetainUntil,
            '--object-lock-legal-hold-status', 'ON'
        )).Output -Context 'Object Lock put-object'
        $context.LockVersion = Get-JsonPropertyString -Object $put -Name VersionId
        Assert-Condition (-not [string]::IsNullOrWhiteSpace([string]$put.ETag)) 'Object Lock PutObject did not return an ETag'
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.LockVersion) -and $context.LockVersion -ne 'null') 'Object Lock PutObject did not return an opaque VersionId'
        $head = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-object -Arguments @('--bucket', $lockBucket, '--key', $context.LockKey, '--version-id', $context.LockVersion)).Output -Context 'Object Lock exact-version HeadObject'
        $expected = ConvertFrom-AwsTimestamp -Value $context.LockRetainUntil -Context 'requested retention'
        $actual = ConvertFrom-AwsTimestamp -Value ([string]$head.ObjectLockRetainUntilDate) -Context 'HeadObject retention'
        Assert-Condition ([string]$head.VersionId -eq $context.LockVersion) 'HeadObject did not address the exact Object Lock version'
        Assert-Condition ([string]$head.ObjectLockMode -eq 'GOVERNANCE') 'HeadObject did not report GOVERNANCE mode'
        Assert-Condition ([string]$head.ObjectLockLegalHoldStatus -eq 'ON') 'HeadObject did not report legal hold ON'
        Assert-Condition ([Math]::Abs(($actual - $expected).TotalSeconds) -le 1) 'HeadObject retain-until date differs from the explicit request'
        'explicit GOVERNANCE retention and legal hold ON are visible on the exact VersionId'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation ObjectLock.Object.GetRetention -Body {
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.LockVersion)) 'Object Lock VersionId is unavailable'
        $result = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object-retention -Arguments @('--bucket', $lockBucket, '--key', $context.LockKey, '--version-id', $context.LockVersion)).Output -Context 'get-object-retention'
        $expected = ConvertFrom-AwsTimestamp -Value $context.LockRetainUntil -Context 'requested retention'
        $actual = ConvertFrom-AwsTimestamp -Value ([string]$result.Retention.RetainUntilDate) -Context 'GetObjectRetention retention'
        Assert-Condition ([string]$result.Retention.Mode -eq 'GOVERNANCE') 'GetObjectRetention mode was not GOVERNANCE'
        Assert-Condition ([Math]::Abs(($actual - $expected).TotalSeconds) -le 1) 'GetObjectRetention retain-until date differs from the explicit request'
        'GetObjectRetention matches the explicit mode and retain-until date'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation ObjectLock.Object.GetLegalHold -Body {
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.LockVersion)) 'Object Lock VersionId is unavailable'
        $result = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object-legal-hold -Arguments @('--bucket', $lockBucket, '--key', $context.LockKey, '--version-id', $context.LockVersion)).Output -Context 'get-object-legal-hold ON'
        Assert-Condition ([string]$result.LegalHold.Status -eq 'ON') 'GetObjectLegalHold did not report ON'
        'GetObjectLegalHold reports ON for the exact VersionId'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation ObjectLock.Object.LegalHoldOff -Body {
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.LockVersion)) 'Object Lock VersionId is unavailable'
        $legalHoldOff = @{ Status = 'OFF' } | ConvertTo-Json -Compress
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation put-object-legal-hold -Arguments @('--bucket', $lockBucket, '--key', $context.LockKey, '--version-id', $context.LockVersion, '--legal-hold', $legalHoldOff) | Out-Null
        $readBack = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation get-object-legal-hold -Arguments @('--bucket', $lockBucket, '--key', $context.LockKey, '--version-id', $context.LockVersion)).Output -Context 'get-object-legal-hold OFF'
        Assert-Condition ([string]$readBack.LegalHold.Status -eq 'OFF') 'legal hold did not become OFF before retention cleanup'
        'legal hold is OFF before any governance-retention delete attempt'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation ObjectLock.Governance.DeleteWithoutBypassDenied -Body {
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.LockVersion)) 'Object Lock VersionId is unavailable'
        $denied = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $lockBucket, '--key', $context.LockKey, '--version-id', $context.LockVersion) -AllowFailure
        Assert-Condition ($denied.ExitCode -ne 0 -and $denied.Output -match '(?i)(AccessDenied|403|Forbidden|governance retention)') "active governance retention was not denied without bypass: $($denied.Output)"
        $head = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-object -Arguments @('--bucket', $lockBucket, '--key', $context.LockKey, '--version-id', $context.LockVersion)).Output -Context 'head after denied no-bypass delete'
        Assert-Condition ([string]$head.VersionId -eq $context.LockVersion) 'denied no-bypass delete removed the exact version'
        'signed delete without bypass is denied and the exact version remains'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation ObjectLock.Governance.UnsignedBypassDenied -Body {
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.LockVersion)) 'Object Lock VersionId is unavailable'
        $denied = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -GlobalArguments @('--no-sign-request') -Arguments @('--bucket', $lockBucket, '--key', $context.LockKey, '--version-id', $context.LockVersion, '--bypass-governance-retention') -AllowFailure
        Assert-Condition ($denied.ExitCode -ne 0 -and $denied.Output -match '(?i)(AccessDenied|403|Forbidden|InvalidAccessKeyId|SignatureDoesNotMatch|MissingAuthentication|Authorization)') "unsigned governance bypass did not produce an authentication or authorization denial: $($denied.Output)"
        $head = ConvertFrom-CompatJson -Json (Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-object -Arguments @('--bucket', $lockBucket, '--key', $context.LockKey, '--version-id', $context.LockVersion)).Output -Context 'head after denied unsigned bypass'
        Assert-Condition ([string]$head.VersionId -eq $context.LockVersion) 'denied unsigned bypass removed the exact version'
        'unsigned bypass is denied and the exact version remains'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation ObjectLock.Governance.BypassDelete -Body {
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($context.LockVersion)) 'Object Lock VersionId is unavailable'
        $deletedVersion = $context.LockVersion
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $lockBucket, '--key', $context.LockKey, '--version-id', $deletedVersion, '--bypass-governance-retention') | Out-Null
        $missing = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-object -Arguments @('--bucket', $lockBucket, '--key', $context.LockKey, '--version-id', $deletedVersion) -AllowFailure
        Assert-Condition ($missing.ExitCode -ne 0 -and $missing.Output -match '(?i)(404|NoSuchKey|NoSuchVersion|Not Found)') 'signed governance bypass returned success but the exact version still exists'
        $context.LockVersion = $null
        'signed bypass deleted the exact retained version'
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation ObjectLock.Bucket.Delete -Body {
        Assert-Condition $context.LockBucketCreated 'Object Lock bucket was not created by this run'
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-bucket -Arguments @('--bucket', $lockBucket) | Out-Null
        $missing = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation head-bucket -Arguments @('--bucket', $lockBucket) -AllowFailure
        Assert-Condition ($missing.ExitCode -ne 0 -and $missing.Output -match '(?i)(404|NoSuchBucket|Not Found)') 'Object Lock bucket remained after DeleteBucket'
        $context.LockBucketCreated = $false
        'empty Object Lock bucket deleted after exact-version cleanup'
    } | Out-Null

    foreach ($upload in $context.MultipartUploads) {
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation abort-multipart-upload -Arguments @('--bucket', $bucket, '--key', $upload.Key, '--upload-id', $upload.UploadId) -AllowFailure | Out-Null
    }
    if (-not [string]::IsNullOrWhiteSpace([string]$context.MultipartVersion)) {
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $bucket, '--key', $multipartKey, '--version-id', $context.MultipartVersion) -AllowFailure | Out-Null
    }
    foreach ($version in @($context.V2, $context.V1, $context.DeleteMarker)) {
        if (-not [string]::IsNullOrWhiteSpace([string]$version)) {
            Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey, '--version-id', $version) -AllowFailure | Out-Null
        }
    }
    Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey, '--version-id', 'null') -AllowFailure | Out-Null
    Invoke-CompatCase -Client aws -Operation Tagging.Cleanup -Body {
        $cleanup = Invoke-AwsTrackedVersionCleanup -Client $Client -Endpoint $Endpoint -Region $Region -Bucket $bucket -Versions $context.TagVersions
        Assert-Condition $cleanup.Success $cleanup.Message
        $context.TagVersions.Clear()
        $cleanup.Message
    } | Out-Null
    Invoke-CompatCase -Client aws -Operation Bucket.Delete -Body {
        Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-bucket -Arguments @('--bucket', $bucket) | Out-Null
        $context.BucketCreated = $false
        'empty bucket deleted'
    } | Out-Null
    }
    finally {
        if ($context.LockBucketCreated) {
            $lockCleanup = Invoke-AwsObjectLockBucketCleanup -Client $Client -Endpoint $Endpoint -Region $Region -Bucket $lockBucket
            if ($lockCleanup.Success) {
                $context.LockBucketCreated = $false
                Add-CompatResult -Client aws -Operation ObjectLock.ClientCleanup -Status PASS -Message $lockCleanup.Message
            }
            else {
                Add-CompatResult -Client aws -Operation ObjectLock.ClientCleanup -Status FAIL -Message ("Object Lock cleanup failed for run bucket ${lockBucket}: {0}" -f $lockCleanup.Message)
            }
        }
        if ($context.BucketCreated) {
            if ($context.TagVersions.Count -gt 0) {
                $tagCleanup = Invoke-AwsTrackedVersionCleanup -Client $Client -Endpoint $Endpoint -Region $Region -Bucket $bucket -Versions $context.TagVersions
                if ($tagCleanup.Success) {
                    $context.TagVersions.Clear()
                    Add-CompatResult -Client aws -Operation Tagging.ClientCleanup -Status PASS -Message $tagCleanup.Message
                }
                else {
                    Add-CompatResult -Client aws -Operation Tagging.ClientCleanup -Status FAIL -Message ("Tagging cleanup failed for run bucket ${bucket}: {0}" -f $tagCleanup.Message)
                }
            }
            foreach ($upload in $context.MultipartUploads) {
                Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation abort-multipart-upload -Arguments @('--bucket', $bucket, '--key', $upload.Key, '--upload-id', $upload.UploadId) -AllowFailure | Out-Null
            }
            if (-not [string]::IsNullOrWhiteSpace([string]$context.MultipartVersion)) {
                Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $bucket, '--key', $multipartKey, '--version-id', $context.MultipartVersion) -AllowFailure | Out-Null
            }
            foreach ($version in @($context.V2, $context.V1, $context.DeleteMarker)) {
                if (-not [string]::IsNullOrWhiteSpace([string]$version)) {
                    Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $bucket, '--key', $context.NullKey, '--version-id', $version) -AllowFailure | Out-Null
                }
            }
            foreach ($key in $knownNullKeys) {
                Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-object -Arguments @('--bucket', $bucket, '--key', $key, '--version-id', 'null') -AllowFailure | Out-Null
            }
            $cleanup = Invoke-AwsApi -Client $Client -Endpoint $Endpoint -Region $Region -Operation delete-bucket -Arguments @('--bucket', $bucket) -AllowFailure
            if ($cleanup.ExitCode -eq 0 -or $cleanup.Output -match '(?i)(NoSuchBucket|Not Found|404)') {
                $context.BucketCreated = $false
                Add-CompatResult -Client aws -Operation Client.Cleanup -Status PASS -Message 'best-effort cleanup removed the test bucket'
            }
            else {
                Add-CompatResult -Client aws -Operation Client.Cleanup -Status FAIL -Message ("test bucket cleanup failed: {0}" -f $cleanup.Output)
            }
        }
    }
}

function Invoke-McliCommand {
    param([object]$Client, [string[]]$Arguments, [switch]$AllowFailure)
    return Invoke-NativeCommand -Command $Client.Path -Arguments $Arguments -AllowFailure:$AllowFailure
}

function Set-McliEnvironment {
    param([string]$Endpoint, [object]$Credential, [string]$ConfigDirectory)
    $uri = [Uri]$Endpoint
    Assert-Condition ([string]::IsNullOrWhiteSpace($uri.Query) -and [string]::IsNullOrWhiteSpace($uri.Fragment)) 'mcli endpoint must not contain query or fragment'
    $user = [Uri]::EscapeDataString([string]$Credential.AccessKeyId)
    $password = [Uri]::EscapeDataString([string]$Credential.SecretAccessKey)
    $path = $uri.AbsolutePath.TrimEnd('/')
    if ($path -eq '/') { $path = '' }
    Set-ScopedEnvironmentVariable MC_HOST_prismark ("{0}://{1}:{2}@{3}{4}" -f $uri.Scheme, $user, $password, $uri.Authority, $path)
    Set-ScopedEnvironmentVariable MC_CONFIG_DIR $ConfigDirectory
}

function Invoke-McliCompatibilityMatrix {
    param(
        [object]$Client,
        [string]$Endpoint,
        [object]$Credential,
        [string]$TempRoot,
        [string]$RunId
    )
    Write-CompatInfo "running $($Client.Name) matrix"
    $work = Join-Path $TempRoot 'mcli'
    New-Item -ItemType Directory -Path $work -Force | Out-Null
    Set-McliEnvironment -Endpoint $Endpoint -Credential $Credential -ConfigDirectory (Join-Path $work 'config')
    $bucket = "prismark-compat-mcli-$RunId"
    $remote = "prismark/$bucket"
    $payloadPath = Join-Path $work 'payload.txt'
    $downloadPath = Join-Path $work 'download.txt'
    $payload = "mcli-payload-$RunId"
    [IO.File]::WriteAllText($payloadPath, $payload, (New-Object Text.UTF8Encoding($false)))
    $clientState = @{ BucketCreated = $false }

    try {
    Invoke-CompatCase -Client mcli -Operation Bucket.Create -Body {
        Invoke-McliCommand -Client $Client -Arguments @('mb', $remote) | Out-Null
        $clientState.BucketCreated = $true
        'bucket created'
    } | Out-Null
    Invoke-CompatCase -Client mcli -Operation Bucket.Head -Body {
        Invoke-McliCommand -Client $Client -Arguments @('stat', $remote) | Out-Null
        'bucket stat succeeded'
    } | Out-Null
    Invoke-CompatCase -Client mcli -Operation Bucket.List -Body {
        $listed = Invoke-McliCommand -Client $Client -Arguments @('ls', 'prismark')
        Assert-Condition ($listed.Output -match [Regex]::Escape($bucket)) 'created bucket was absent from mcli ls'
        'bucket appears in root listing'
    } | Out-Null
    Invoke-CompatCase -Client mcli -Operation Object.Put -Body {
        Invoke-McliCommand -Client $Client -Arguments @('cp', '--quiet', $payloadPath, "$remote/basic.txt") | Out-Null
        'object uploaded'
    } | Out-Null
    Invoke-CompatCase -Client mcli -Operation Object.Head -Body {
        Invoke-McliCommand -Client $Client -Arguments @('stat', "$remote/basic.txt") | Out-Null
        'object stat succeeded'
    } | Out-Null
    Invoke-CompatCase -Client mcli -Operation Object.Get -Body {
        Invoke-McliCommand -Client $Client -Arguments @('cp', '--quiet', "$remote/basic.txt", $downloadPath) | Out-Null
        Assert-Condition ((Get-FileSha256 $payloadPath) -eq (Get-FileSha256 $downloadPath)) 'mcli download checksum mismatch'
        'downloaded body checksum matches'
    } | Out-Null
    Invoke-CompatCase -Client mcli -Operation Object.Range -Body {
        $range = Invoke-McliCommand -Client $Client -Arguments @('cat', '--offset', '2', '--length', '6', "$remote/basic.txt")
        Assert-Condition ($range.Output -eq $payload.Substring(2, 6)) 'mcli range output mismatch'
        'offset/length range matches'
    } | Out-Null
    Add-SkipResult -Client mcli -Operation Object.Conditional -Reason 'mcli has no stable direct If-Match/If-None-Match command surface; AWS CLI covers this protocol operation'
    Add-SkipResult -Client mcli -Operation CopyObject -Reason 'mcli cp does not expose a stable assertion that the server used CopyObject rather than a client-side transfer; AWS CLI performs the protocol probe'
    Add-SkipResult -Client mcli -Operation UploadPartCopy -Reason 'mcli has no direct UploadPartCopy command; AWS CLI performs the low-level protocol probe'

    foreach ($key in @('dir/a.txt', 'dir/sub/b.txt')) {
        Invoke-McliCommand -Client $Client -Arguments @('cp', '--quiet', $payloadPath, "$remote/$key") | Out-Null
    }
    Invoke-CompatCase -Client mcli -Operation ListObjectsV2.PrefixDelimiter -Body {
        $listed = Invoke-McliCommand -Client $Client -Arguments @('ls', "$remote/dir/")
        Assert-Condition ($listed.Output -match 'a\.txt' -and $listed.Output -match 'sub/') 'mcli prefix listing omitted an object or common prefix'
        'high-level prefix listing returns file and directory'
    } | Out-Null
    Add-SkipResult -Client mcli -Operation ListObjectsV2.Pagination -Reason 'mcli does not expose max-keys/continuation-token controls; AWS CLI covers explicit pagination'
    Add-SkipResult -Client mcli -Operation DeleteObjects -Reason 'mcli rm does not guarantee an observable DeleteObjects request; AWS CLI covers the batch API'

    Invoke-CompatCase -Client mcli -Operation Object.Delete -Body {
        Invoke-McliCommand -Client $Client -Arguments @('rm', '--recursive', '--force', "$remote/") | Out-Null
        'test objects deleted'
    } | Out-Null
    Invoke-CompatCase -Client mcli -Operation Versioning.Enable -Body {
        Invoke-McliCommand -Client $Client -Arguments @('version', 'enable', $remote) | Out-Null
        $info = Invoke-McliCommand -Client $Client -Arguments @('version', 'info', $remote)
        Assert-Condition ($info.Output -match '(?i)enabled') 'mcli did not report versioning as enabled'
        'bucket versioning enabled through mcli'
    } | Out-Null
    Add-SkipResult -Client mcli -Operation Versioning.NullVersion -Reason 'mcli does not expose deterministic versionId=null addressing for this matrix'
    Add-SkipResult -Client mcli -Operation Versioning.DeleteMarker -Reason 'mcli does not expose a stable cross-version delete-marker assertion for this matrix; AWS CLI verifies it directly'
    Add-SkipResult -Client mcli -Operation Versioning.ExactVersion -Reason 'mcli exact-version discovery is not deterministic enough for this matrix; AWS CLI verifies known version IDs directly'
    Add-SkipResult -Client mcli -Operation Multipart.LowLevel -Reason 'mcli has no low-level create/upload-part/list-parts/complete/abort command set; AWS CLI covers each operation'
    Add-SkipResult -Client mcli -Operation MultipartUploads.List -Reason 'mcli has no stable low-level ListMultipartUploads command for this matrix'
    Add-SkipResult -Client mcli -Operation ObjectVersions.List -Reason 'mcli does not expose a deterministic raw ListObjectVersions assertion; AWS CLI verifies the protocol directly'
    Invoke-CompatCase -Client mcli -Operation Bucket.Delete -Body {
        Invoke-McliCommand -Client $Client -Arguments @('rb', $remote) | Out-Null
        $clientState.BucketCreated = $false
        'empty versioned bucket deleted'
    } | Out-Null
    }
    finally {
        if ($clientState.BucketCreated) {
            Invoke-McliCommand -Client $Client -Arguments @('rm', '--recursive', '--force', "$remote/") -AllowFailure | Out-Null
            $cleanup = Invoke-McliCommand -Client $Client -Arguments @('rb', $remote) -AllowFailure
            if ($cleanup.ExitCode -eq 0 -or $cleanup.Output -match '(?i)(not found|does not exist|NoSuchBucket)') {
                Add-CompatResult -Client mcli -Operation Client.Cleanup -Status PASS -Message 'best-effort cleanup removed the test bucket'
            }
            else {
                Add-CompatResult -Client mcli -Operation Client.Cleanup -Status FAIL -Message ("test bucket cleanup failed: {0}" -f $cleanup.Output)
            }
        }
    }
}

function Invoke-RcloneCommand {
    param([object]$Client, [string[]]$Arguments, [switch]$AllowFailure)
    return Invoke-NativeCommand -Command $Client.Path -Arguments $Arguments -AllowFailure:$AllowFailure
}

function Set-RcloneEnvironment {
    param([string]$Endpoint, [string]$Region, [object]$Credential)
    Set-ScopedEnvironmentVariable AWS_ACCESS_KEY_ID $Credential.AccessKeyId
    Set-ScopedEnvironmentVariable AWS_SECRET_ACCESS_KEY $Credential.SecretAccessKey
    Set-ScopedEnvironmentVariable AWS_EC2_METADATA_DISABLED 'true'
    Set-ScopedEnvironmentVariable RCLONE_CONFIG_PRISMARK_TYPE 's3'
    Set-ScopedEnvironmentVariable RCLONE_CONFIG_PRISMARK_PROVIDER 'Other'
    Set-ScopedEnvironmentVariable RCLONE_CONFIG_PRISMARK_ENV_AUTH 'true'
    Set-ScopedEnvironmentVariable RCLONE_CONFIG_PRISMARK_ENDPOINT $Endpoint
    Set-ScopedEnvironmentVariable RCLONE_CONFIG_PRISMARK_REGION $Region
    Set-ScopedEnvironmentVariable RCLONE_CONFIG_PRISMARK_FORCE_PATH_STYLE 'true'
    Set-ScopedEnvironmentVariable RCLONE_CONFIG_PRISMARK_LIST_CHUNK '1'
    Set-ScopedEnvironmentVariable RCLONE_CONFIG_PRISMARK_UPLOAD_CUTOFF '5Mi'
    Set-ScopedEnvironmentVariable RCLONE_CONFIG_PRISMARK_CHUNK_SIZE '5Mi'
    Set-ScopedEnvironmentVariable RCLONE_CONFIG_PRISMARK_DELETE_BATCH_SIZE '100'
}

function Invoke-RcloneCompatibilityMatrix {
    param(
        [object]$Client,
        [string]$Endpoint,
        [string]$Region,
        [object]$Credential,
        [string]$TempRoot,
        [string]$RunId
    )
    Write-CompatInfo 'running rclone matrix'
    Set-RcloneEnvironment -Endpoint $Endpoint -Region $Region -Credential $Credential
    $work = Join-Path $TempRoot 'rclone'
    New-Item -ItemType Directory -Path $work -Force | Out-Null
    $bucket = "prismark-compat-rclone-$RunId"
    $remote = "prismark:$bucket"
    $payloadPath = Join-Path $work 'payload.txt'
    $downloadPath = Join-Path $work 'download.txt'
    $payload = "rclone-payload-$RunId"
    [IO.File]::WriteAllText($payloadPath, $payload, (New-Object Text.UTF8Encoding($false)))
    $clientState = @{ BucketCreated = $false }

    try {
    Invoke-CompatCase -Client rclone -Operation Bucket.Create -Body {
        Invoke-RcloneCommand -Client $Client -Arguments @('mkdir', $remote) | Out-Null
        $clientState.BucketCreated = $true
        'bucket created'
    } | Out-Null
    Invoke-CompatCase -Client rclone -Operation Bucket.Head -Body {
        Invoke-RcloneCommand -Client $Client -Arguments @('lsf', "$remote/", '--max-depth', '1') | Out-Null
        'bucket can be addressed and listed'
    } | Out-Null
    Invoke-CompatCase -Client rclone -Operation Bucket.List -Body {
        $listed = Invoke-RcloneCommand -Client $Client -Arguments @('lsd', 'prismark:')
        Assert-Condition ($listed.Output -match [Regex]::Escape($bucket)) 'created bucket was absent from rclone lsd'
        'bucket appears in root listing'
    } | Out-Null
    Invoke-CompatCase -Client rclone -Operation Object.Put -Body {
        Invoke-RcloneCommand -Client $Client -Arguments @('copyto', $payloadPath, "$remote/basic.txt") | Out-Null
        'object uploaded'
    } | Out-Null
    Invoke-CompatCase -Client rclone -Operation Object.Head -Body {
        $listed = Invoke-RcloneCommand -Client $Client -Arguments @('lsjson', "$remote/", '--files-only', '--include', '/basic.txt')
        $json = ConvertFrom-CompatJson -Json $listed.Output -Context 'rclone lsjson'
        Assert-Condition (@($json).Count -eq 1 -and [long]$json[0].Size -eq ([IO.FileInfo]$payloadPath).Length) 'rclone object size mismatch'
        'object metadata size matches'
    } | Out-Null
    Invoke-CompatCase -Client rclone -Operation Object.Get -Body {
        Invoke-RcloneCommand -Client $Client -Arguments @('copyto', "$remote/basic.txt", $downloadPath) | Out-Null
        Assert-Condition ((Get-FileSha256 $payloadPath) -eq (Get-FileSha256 $downloadPath)) 'rclone download checksum mismatch'
        'downloaded body checksum matches'
    } | Out-Null
    Invoke-CompatCase -Client rclone -Operation Object.Range -Body {
        $range = Invoke-RcloneCommand -Client $Client -Arguments @('cat', "$remote/basic.txt", '--offset', '2', '--count', '6')
        Assert-Condition ($range.Output -eq $payload.Substring(2, 6)) 'rclone range output mismatch'
        'offset/count range matches'
    } | Out-Null
    Add-SkipResult -Client rclone -Operation Object.Conditional -Reason 'rclone does not expose direct If-Match/If-None-Match flags; AWS CLI covers the conditional API'
    Add-SkipResult -Client rclone -Operation CopyObject -Reason 'rclone may choose server-side copy internally but does not provide a deterministic CopyObject protocol probe'
    Add-SkipResult -Client rclone -Operation UploadPartCopy -Reason 'rclone has no direct low-level UploadPartCopy command'

    foreach ($key in @('dir/a.txt', 'dir/sub/b.txt', 'page/1.txt', 'page/2.txt', 'page/3.txt', 'batch/1.txt', 'batch/2.txt', 'batch/3.txt')) {
        Invoke-RcloneCommand -Client $Client -Arguments @('copyto', $payloadPath, "$remote/$key") | Out-Null
    }
    Invoke-CompatCase -Client rclone -Operation ListObjectsV2.PrefixDelimiter -Body {
        $listed = Invoke-RcloneCommand -Client $Client -Arguments @('lsf', "$remote/dir/", '--max-depth', '1')
        Assert-Condition ($listed.Output -match '(?m)^a\.txt$' -and $listed.Output -match '(?m)^sub/$') 'rclone prefix listing omitted an object or common prefix'
        'non-recursive prefix listing returns file and directory'
    } | Out-Null
    Invoke-CompatCase -Client rclone -Operation ListObjectsV2.Pagination -Body {
        $listed = Invoke-RcloneCommand -Client $Client -Arguments @('lsf', "$remote/page/", '--recursive', '--files-only')
        foreach ($name in @('1.txt', '2.txt', '3.txt')) {
            Assert-Condition ($listed.Output -match ("(?m)^{0}$" -f [Regex]::Escape($name))) "rclone paginated listing omitted $name"
        }
        'list_chunk=1 forced continuation-token pagination across three objects'
    } | Out-Null
    Invoke-CompatCase -Client rclone -Operation DeleteObjects -Body {
        Invoke-RcloneCommand -Client $Client -Arguments @('delete', "$remote/batch/", '--rmdirs') | Out-Null
        $remaining = Invoke-RcloneCommand -Client $Client -Arguments @('lsf', "$remote/", '--recursive', '--files-only')
        Assert-Condition ($remaining.Output -notmatch '(?m)^batch/') 'rclone batch delete left objects behind'
        'rclone S3 backend batch deletion removed all batch objects'
    } | Out-Null

    $largePath = Join-Path $work 'multipart.bin'
    New-TestBytesFile -Path $largePath -Bytes (6 * 1024 * 1024)
    Invoke-CompatCase -Client rclone -Operation Multipart.Automatic -Body {
        Invoke-RcloneCommand -Client $Client -Arguments @('copyto', $largePath, "$remote/multipart.bin") | Out-Null
        $multipartDownload = Join-Path $work 'multipart-download.bin'
        Invoke-RcloneCommand -Client $Client -Arguments @('copyto', "$remote/multipart.bin", $multipartDownload) | Out-Null
        Assert-Condition ((Get-FileSha256 $largePath) -eq (Get-FileSha256 $multipartDownload)) 'rclone automatic multipart checksum mismatch'
        '5 MiB upload cutoff forced automatic multipart and checksum matches'
    } | Out-Null
    Add-SkipResult -Client rclone -Operation Multipart.ListParts -Reason 'rclone does not expose ListParts as a direct command'
    Add-SkipResult -Client rclone -Operation Multipart.Abort -Reason 'rclone does not expose deterministic low-level AbortMultipartUpload control'
    Add-SkipResult -Client rclone -Operation MultipartUploads.List -Reason 'rclone does not expose ListMultipartUploads as a direct command'
    Add-SkipResult -Client rclone -Operation Versioning.Enable -Reason 'rclone does not expose PutBucketVersioning as a direct command'
    Add-SkipResult -Client rclone -Operation Versioning.NullVersion -Reason 'rclone does not expose deterministic versionId=null addressing'
    Add-SkipResult -Client rclone -Operation Versioning.DeleteMarker -Reason 'rclone does not expose a deterministic delete-marker assertion; AWS CLI verifies it directly'
    Add-SkipResult -Client rclone -Operation Versioning.ExactVersion -Reason 'rclone does not expose deterministic exact-version mutation for this matrix'
    Add-SkipResult -Client rclone -Operation ObjectVersions.List -Reason 'rclone does not expose a deterministic raw ListObjectVersions assertion; AWS CLI performs the protocol probe'
    Invoke-CompatCase -Client rclone -Operation Object.Delete -Body {
        Invoke-RcloneCommand -Client $Client -Arguments @('delete', "$remote/", '--rmdirs') | Out-Null
        'all test objects deleted'
    } | Out-Null
    Invoke-CompatCase -Client rclone -Operation Bucket.Delete -Body {
        Invoke-RcloneCommand -Client $Client -Arguments @('rmdir', $remote) | Out-Null
        $clientState.BucketCreated = $false
        'empty bucket deleted'
    } | Out-Null
    }
    finally {
        if ($clientState.BucketCreated) {
            Invoke-RcloneCommand -Client $Client -Arguments @('delete', "$remote/", '--rmdirs') -AllowFailure | Out-Null
            $cleanup = Invoke-RcloneCommand -Client $Client -Arguments @('rmdir', $remote) -AllowFailure
            if ($cleanup.ExitCode -eq 0 -or $cleanup.Output -match '(?i)(not found|directory not found|NoSuchBucket)') {
                Add-CompatResult -Client rclone -Operation Client.Cleanup -Status PASS -Message 'best-effort cleanup removed the test bucket'
            }
            else {
                Add-CompatResult -Client rclone -Operation Client.Cleanup -Status FAIL -Message ("test bucket cleanup failed: {0}" -f $cleanup.Output)
            }
        }
    }
}
