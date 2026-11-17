[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $GeneratorPath,

    [string] $ExpectedSha256,

    [switch] $AdvancePin
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$SourceUrl = "https://iseyon.mcp-staging.iseyon.com/openapi.json"
$ExpectedOpenApiVersion = "3.1.0"
$ExpectedTitle = "IDA MCP Server"
$ExpectedVersion = "0.1.10"
$PinnedSha256 = "048be51a7eab7f651f9f801ed30017f88daf9b33542228e501b6d85fa5ba7ac4"
$DaSourceRevision = "beb7ebcfa2ff43ddf4e0ebd657c460755abae940"
$GeneratorVersion = "0.26.2"

function Write-Utf8NoBomLf {
    param(
        [Parameter(Mandatory = $true)]
        [string] $LiteralPath,

        [Parameter(Mandatory = $true)]
        [string] $Content
    )

    $normalized = $Content.Replace("`r`n", "`n").Replace("`r", "`n")
    [System.IO.File]::WriteAllText(
        $LiteralPath,
        $normalized,
        [System.Text.UTF8Encoding]::new($false)
    )
}

if (-not (Test-Path -LiteralPath $GeneratorPath -PathType Leaf)) {
    throw "oas3-gen executable not found at '$GeneratorPath'. Pass the explicit pinned 0.26.2 executable path."
}

$generatorVersionOutput = (& $GeneratorPath --version 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $generatorVersionOutput -ne "oas3-gen $GeneratorVersion") {
    throw "Expected oas3-gen $GeneratorVersion, received '$generatorVersionOutput'."
}

$rustfmtCommand = Get-Command rustfmt -CommandType Application -ErrorAction SilentlyContinue
if ($null -eq $rustfmtCommand) {
    throw "rustfmt is required to canonicalize generated Rust output."
}

$sourceUri = [System.Uri]::new($SourceUrl)
if ($sourceUri.Scheme -ne [System.Uri]::UriSchemeHttps -or $sourceUri.AbsoluteUri -ne $SourceUrl) {
    throw "The DA contract source must be the exact pinned HTTPS URL '$SourceUrl'."
}

$expectedWasExplicit = $PSBoundParameters.ContainsKey("ExpectedSha256")
if ($expectedWasExplicit) {
    if ([string]::IsNullOrWhiteSpace($ExpectedSha256)) {
        throw "ExpectedSha256 must be a non-empty SHA-256 value."
    }
    $targetSha256 = $ExpectedSha256.ToLowerInvariant()
}
else {
    $targetSha256 = $PinnedSha256
}

if ($targetSha256 -notmatch "^[0-9a-f]{64}$") {
    throw "ExpectedSha256 must be exactly 64 lowercase hexadecimal characters."
}

if ($targetSha256 -ne $PinnedSha256) {
    if (-not $AdvancePin) {
        throw "Changing the reviewed DA contract hash requires -AdvancePin and an explicit -ExpectedSha256 value."
    }
    throw "ExpectedSha256 differs from the reviewed script pin. Update PinnedSha256 in this script in the same reviewed change."
}

if ($AdvancePin -and -not $expectedWasExplicit) {
    throw "-AdvancePin requires an explicit -ExpectedSha256 value."
}

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$contractRoot = Join-Path $repositoryRoot "contracts/da"
$openApiPath = Join-Path $contractRoot "openapi.json"
$provenancePath = Join-Path $contractRoot "provenance.json"
$shaSumsPath = Join-Path $contractRoot "SHA256SUMS"
$readmePath = Join-Path $contractRoot "README.md"
$generatedPath = Join-Path $repositoryRoot "src-tauri/src/clients/da/generated.rs"

$existingSha256 = $null
$retrievedAt = $null
if (Test-Path -LiteralPath $provenancePath -PathType Leaf) {
    $existingProvenance = Get-Content -Raw -LiteralPath $provenancePath |
        ConvertFrom-Json -DateKind String
    $existingSha256 = [string] $existingProvenance.sha256
    $retrievedAt = [string] $existingProvenance.retrieved_at
    $parsedRetrievalTimestamp = [System.DateTimeOffset]::MinValue
    $hasCanonicalRetrievalTimestamp = [System.DateTimeOffset]::TryParseExact(
        $retrievedAt,
        "o",
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::RoundtripKind,
        [ref] $parsedRetrievalTimestamp
    )
    if (-not $hasCanonicalRetrievalTimestamp) {
        $retrievedAt = $null
    }
}
elseif (Test-Path -LiteralPath $openApiPath -PathType Leaf) {
    $existingSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $openApiPath).Hash.ToLowerInvariant()
}

if ($null -ne $existingSha256 -and $existingSha256 -ne $targetSha256 -and -not $AdvancePin) {
    throw "Changing the committed DA contract pin requires -AdvancePin and an explicit -ExpectedSha256 value."
}

$temporaryDirectory = [System.IO.Path]::GetTempPath()
$temporaryToken = [System.IO.Path]::GetRandomFileName()
$temporarySpec = Join-Path $temporaryDirectory "$temporaryToken-openapi.json"
$temporaryGenerated = Join-Path $temporaryDirectory "$temporaryToken-generated.rs"

try {
    Invoke-WebRequest `
        -Uri $SourceUrl `
        -OutFile $temporarySpec `
        -MaximumRedirection 0

    $actualSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $temporarySpec).Hash.ToLowerInvariant()
    if ($actualSha256 -ne $targetSha256) {
        throw "DA contract SHA-256 mismatch. Expected '$targetSha256', received '$actualSha256'."
    }

    $document = Get-Content -Raw -LiteralPath $temporarySpec | ConvertFrom-Json -Depth 100
    if ([string] $document.openapi -ne $ExpectedOpenApiVersion) {
        throw "DA contract OpenAPI version mismatch. Expected '$ExpectedOpenApiVersion', received '$($document.openapi)'."
    }
    if ([string] $document.info.title -ne $ExpectedTitle) {
        throw "DA contract title mismatch. Expected '$ExpectedTitle', received '$($document.info.title)'."
    }
    if ([string] $document.info.version -ne $ExpectedVersion) {
        throw "DA contract version mismatch. Expected '$ExpectedVersion', received '$($document.info.version)'."
    }

    & $GeneratorPath generate `
        --input $temporarySpec `
        --output $temporaryGenerated `
        --visibility crate `
        --enum-layout sorted `
        --all-schemas `
        --quiet
    if ($LASTEXITCODE -ne 0) {
        throw "oas3-gen failed with exit code $LASTEXITCODE."
    }

    $generated = Get-Content -Raw -LiteralPath $temporaryGenerated
    $canonicalSourceHeader = "//! Source: $SourceUrl`n//! Source SHA-256: $actualSha256"
    $generated = [System.Text.RegularExpressions.Regex]::Replace(
        $generated,
        "(?m)^//! Source: .*$",
        $canonicalSourceHeader
    )
    $generated = [System.Text.RegularExpressions.Regex]::Replace(
        $generated,
        "\bDebug,\s*",
        ""
    )
    if ($generated -notmatch "(?m)^#!\[allow\(non_camel_case_types\)\]$") {
        $generated = "#![allow(non_camel_case_types)]`n$generated"
    }
    if ($generated -notmatch "(?m)^#!\[allow\(clippy::needless_return\)\]$") {
        $generated = $generated -replace "(?m)^(#!\[allow\(clippy::missing_panics_doc\)\]\r?\n)", "`$1#![allow(clippy::needless_return)]`n"
    }

    if ($generated -notmatch [System.Text.RegularExpressions.Regex]::Escape($canonicalSourceHeader)) {
        throw "Generated output did not contain the canonical DA source header."
    }
    if ([System.Text.RegularExpressions.Regex]::IsMatch($generated, "\bDebug\b")) {
        $remainingDebug = [System.Text.RegularExpressions.Regex]::Match($generated, "\bDebug\b")
        $contextStart = [System.Math]::Max(0, $remainingDebug.Index - 80)
        $contextLength = [System.Math]::Min(200, $generated.Length - $contextStart)
        $remainingContext = $generated.Substring($contextStart, $contextLength)
        throw "Generated output still contains Debug after derive hardening near: $remainingContext"
    }

    Write-Utf8NoBomLf -LiteralPath $temporaryGenerated -Content $generated
    & $rustfmtCommand.Source --edition 2021 $temporaryGenerated
    if ($LASTEXITCODE -ne 0) {
        throw "rustfmt failed with exit code $LASTEXITCODE."
    }
    $generated = Get-Content -Raw -LiteralPath $temporaryGenerated

    if ($actualSha256 -ne $existingSha256 -or [string]::IsNullOrWhiteSpace($retrievedAt)) {
        $retrievedAt = [System.DateTimeOffset]::UtcNow.ToString("o")
    }

    $provenanceJson = @'
{
  "source_url": "{SOURCE_URL}",
  "openapi_version": "{OPENAPI_VERSION}",
  "title": "{TITLE}",
  "version": "{VERSION}",
  "sha256": "{SHA256}",
  "da_source_revision": "{DA_SOURCE_REVISION}",
  "retrieved_at": "{RETRIEVED_AT}",
  "generator": {
    "name": "oas3-gen",
    "version": "{GENERATOR_VERSION}",
    "mode": "types",
    "arguments": ["--visibility crate", "--enum-layout sorted", "--all-schemas"],
    "postprocess": "rustfmt --edition 2021"
  }
}
'@
    $provenanceJson = $provenanceJson.
        Replace("{SOURCE_URL}", $SourceUrl).
        Replace("{OPENAPI_VERSION}", $ExpectedOpenApiVersion).
        Replace("{TITLE}", $ExpectedTitle).
        Replace("{VERSION}", $ExpectedVersion).
        Replace("{SHA256}", $actualSha256).
        Replace("{DA_SOURCE_REVISION}", $DaSourceRevision).
        Replace("{RETRIEVED_AT}", $retrievedAt).
        Replace("{GENERATOR_VERSION}", $GeneratorVersion) + "`n"

    $readme = @'
# DA OpenAPI contract

This directory contains the complete, immutable OpenAPI 3.1 snapshot used to
generate xFalcon's crate-private DA DTOs.

- Source: `{SOURCE_URL}`
- OpenAPI: `{OPENAPI_VERSION}`
- API title: `{TITLE}`
- API version: `{VERSION}`
- SHA-256: `{SHA256}`
- DA source revision: `{DA_SOURCE_REVISION}`
- Generator: `oas3-gen {GENERATOR_VERSION}`
- Post-processing: `rustfmt --edition 2021`

Regenerate from the repository root with an explicit path to the pinned
generator:

```powershell
pwsh ./scripts/sync-da-contract.ps1 -GeneratorPath <path-to-oas3-gen-0.26.2>
```

The script validates the exact source, metadata, generator version, and SHA-256
before replacing any committed artifact. To advance the immutable pin, update
`PinnedSha256` in the script in the same reviewed change, then invoke the script
with both `-AdvancePin` and an explicit `-ExpectedSha256`.

The generated Rust output is intentionally crate-private and has all `Debug`
derives removed so secret-bearing DA models cannot be formatted into logs.
'@
    $readme = $readme.
        Replace("{SOURCE_URL}", $SourceUrl).
        Replace("{OPENAPI_VERSION}", $ExpectedOpenApiVersion).
        Replace("{TITLE}", $ExpectedTitle).
        Replace("{VERSION}", $ExpectedVersion).
        Replace("{SHA256}", $actualSha256).
        Replace("{DA_SOURCE_REVISION}", $DaSourceRevision).
        Replace("{GENERATOR_VERSION}", $GeneratorVersion)

    New-Item -ItemType Directory -Force -Path $contractRoot | Out-Null
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $generatedPath) | Out-Null

    [System.IO.File]::WriteAllBytes(
        $openApiPath,
        [System.IO.File]::ReadAllBytes($temporarySpec)
    )
    Write-Utf8NoBomLf -LiteralPath $provenancePath -Content $provenanceJson
    Write-Utf8NoBomLf -LiteralPath $shaSumsPath -Content "$actualSha256  openapi.json`n"
    Write-Utf8NoBomLf -LiteralPath $readmePath -Content ($readme + "`n")
    Write-Utf8NoBomLf -LiteralPath $generatedPath -Content $generated

    Write-Output "Pinned DA OpenAPI $ExpectedVersion ($actualSha256)."
    Write-Output "Generated crate-private Rust DTOs with oas3-gen $GeneratorVersion."
}
finally {
    if (Test-Path -LiteralPath $temporarySpec) {
        Remove-Item -LiteralPath $temporarySpec -Force
    }
    if (Test-Path -LiteralPath $temporaryGenerated) {
        Remove-Item -LiteralPath $temporaryGenerated -Force
    }
}
