#requires -Version 5.1
<#
.SYNOPSIS
    Generates a synthetic project directory tree for exercising
    `crate::projects::index`/`search`/`read` at scale (Task 5's "100,000-file
    synthetic tree startup/search behavior" requirement).

.DESCRIPTION
    Written in PowerShell, not a shell script or a Rust `#[bench]`/xtask,
    because this repository's toolchain is Windows `cargo.exe` invoked from a
    WSL shell (see `docs/engineering/reference-evidence/`), and every
    supported desktop platform for this benchmark's actual use (a developer
    or CI runner exercising `xfalcon-desktop` on Windows) has PowerShell
    available without an extra dependency. `README.md` in this directory
    explains what the resulting tree is for and how to point the app/tests
    at it.

    The generated tree deliberately mixes the same shapes the plan's Task 5
    test matrix (`src-tauri/src/projects/tests/index.rs`) exercises one file
    at a time, but at volume:
      - plain text files of varying size (the common case `search_text`
        scans);
      - a `.gitignore` that excludes a real fraction of the tree, so a
        startup pass has genuine ignore-matching work to do, not just I/O;
      - a small number of binary files (never text-searchable);
      - nested directories (`-MaxDepth`/`-DirsPerLevel`) so the walk exercises
        real directory-traversal fan-out, not one flat directory;
      - (opt-in, `-IncludeSensitiveSamples`) a handful of files named after
        `crate::projects::sensitive::SecretClassifier`'s built-in deny
        classes (e.g. `.env`, `id_rsa`), so a benchmark run can also smoke-test
        that classification and the search/read override gate behave
        correctly at realistic tree scale, not only in the unit fixtures.

    This script only ever writes under `-Root`; nothing here reads, trusts,
    or registers the generated directory with the app on its own -- pointing
    xfalcon-desktop's "trust a project" flow at the resulting path is a
    separate, manual step (see README.md).

.PARAMETER Root
    Directory to generate the tree under. Created if missing. Must not be
    the repository root or any existing trusted project -- this script does
    not check that for you.

.PARAMETER FileCount
    Total number of regular files to generate across the tree. Defaults to
    100,000 to match the plan's own benchmark scale.

.PARAMETER MaxDepth
    Maximum nesting depth of generated subdirectories.

.PARAMETER DirsPerLevel
    Number of subdirectories created at each level up to -MaxDepth.

.PARAMETER IncludeSensitiveSamples
    Also writes a small, fixed set of files matching
    `SecretClassifier`'s built-in classes (never real secrets -- placeholder
    content only) so a benchmark run can be paired with a manual
    `project.read_file`/`project.search_text` smoke check of the sensitive
    override gate at scale.

.PARAMETER Clean
    Remove -Root entirely before generating, if it already exists.

.EXAMPLE
    ./generate-tree.ps1 -Root C:\bench\xfalcon-project-index -FileCount 100000

.EXAMPLE
    ./generate-tree.ps1 -Root C:\bench\smoke -FileCount 2000 -IncludeSensitiveSamples
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Root,

    [int]$FileCount = 100000,

    [int]$MaxDepth = 6,

    [int]$DirsPerLevel = 8,

    [switch]$IncludeSensitiveSamples,

    [switch]$Clean
)

$ErrorActionPreference = 'Stop'

if ($FileCount -le 0) {
    throw "-FileCount must be positive (got $FileCount)"
}
if ($MaxDepth -lt 0) {
    throw "-MaxDepth must not be negative (got $MaxDepth)"
}
if ($DirsPerLevel -le 0) {
    throw "-DirsPerLevel must be positive (got $DirsPerLevel)"
}

if ($Clean -and (Test-Path -LiteralPath $Root)) {
    Write-Host "Removing existing tree at $Root ..."
    Remove-Item -LiteralPath $Root -Recurse -Force
}

New-Item -ItemType Directory -Path $Root -Force | Out-Null
$Root = (Resolve-Path -LiteralPath $Root).Path

# ---------------------------------------------------------------------------
# Build the directory list up front (breadth-first, bounded fan-out) so file
# generation below can index straight into it rather than re-walking or
# re-creating directories per file.
# ---------------------------------------------------------------------------
$directories = [System.Collections.Generic.List[string]]::new()
$directories.Add($Root)

$frontier = [System.Collections.Generic.List[string]]::new()
$frontier.Add($Root)
for ($depth = 0; $depth -lt $MaxDepth; $depth++) {
    $nextFrontier = [System.Collections.Generic.List[string]]::new()
    foreach ($parent in $frontier) {
        for ($i = 0; $i -lt $DirsPerLevel; $i++) {
            $child = Join-Path $parent ("dir_{0:D2}" -f $i)
            [void][System.IO.Directory]::CreateDirectory($child)
            $directories.Add($child)
            $nextFrontier.Add($child)
        }
    }
    $frontier = $nextFrontier
}

Write-Host ("Created {0} directories (max depth {1}, {2} per level)." -f $directories.Count, $MaxDepth, $DirsPerLevel)

# ---------------------------------------------------------------------------
# Ignore rules: a real `.gitignore` at the root so `ignore::WalkBuilder`
# (`crate::projects::index`) has genuine pattern-matching work, not just
# raw I/O. `*.ignored` files are generated below and must never appear in an
# index/search result.
# ---------------------------------------------------------------------------
$gitignorePath = Join-Path $Root '.gitignore'
@(
    '*.ignored'
    'node_modules/'
    '.build-cache/'
) -join "`n" | Set-Content -LiteralPath $gitignorePath -NoNewline -Encoding UTF8

# ---------------------------------------------------------------------------
# File generation. Deterministic content sizes drawn from a fixed cycle
# (not `Get-Random`) so repeated runs with the same -FileCount produce a
# reproducible byte total for comparing benchmark runs across machines/dates.
# ---------------------------------------------------------------------------
$sizeCycleBytes = @(64, 512, 4096, 65536, 262144) # 64B .. 256KiB
$textLine = [System.Text.Encoding]::UTF8.GetBytes(
    "the quick brown fox jumps over the lazy dog line for xfalcon project index benchmark`n"
)

$binaryEveryNth = 500   # ~0.2% of files are binary
$ignoredEveryNth = 200  # ~0.5% of files are `.gitignore`-excluded

$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
$progressEvery = [Math]::Max(1, [int]($FileCount / 20))

for ($i = 0; $i -lt $FileCount; $i++) {
    $directory = $directories[$i % $directories.Count]
    $sizeTarget = $sizeCycleBytes[$i % $sizeCycleBytes.Length]

    if ($i % $ignoredEveryNth -eq 0) {
        $path = Join-Path $directory ("scratch_{0:D7}.ignored" -f $i)
        [System.IO.File]::WriteAllBytes($path, (New-Object byte[] $sizeTarget))
        continue
    }

    if ($i % $binaryEveryNth -eq 0) {
        $path = Join-Path $directory ("asset_{0:D7}.bin" -f $i)
        $bytes = New-Object byte[] $sizeTarget
        # A single embedded NUL byte is sufficient for
        # `crate::projects::read::hash_and_classify`'s whole-buffer scan to
        # classify this as binary, matching real binary assets (images,
        # archives) rather than requiring a full random fill.
        if ($bytes.Length -gt 0) { $bytes[0] = 0 }
        [System.IO.File]::WriteAllBytes($path, $bytes)
        continue
    }

    $path = Join-Path $directory ("source_{0:D7}.txt" -f $i)
    $repeats = [Math]::Max(1, [int]($sizeTarget / $textLine.Length))
    $stream = [System.IO.File]::Open($path, [System.IO.FileMode]::Create)
    try {
        for ($line = 0; $line -lt $repeats; $line++) {
            $stream.Write($textLine, 0, $textLine.Length)
        }
    }
    finally {
        $stream.Dispose()
    }

    if (($i + 1) % $progressEvery -eq 0) {
        $elapsed = $stopwatch.Elapsed.TotalSeconds
        Write-Host ("  {0}/{1} files written ({2:N1}s elapsed)" -f ($i + 1), $FileCount, $elapsed)
    }
}

if ($IncludeSensitiveSamples) {
    Write-Host "Writing sensitive-class sample files (placeholder content only) ..."
    $sensitiveSamples = @{
        '.env'                     = "PLACEHOLDER_SECRET=not-a-real-value`n"
        'id_rsa'                   = "-----BEGIN OPENSSH PRIVATE KEY-----`nPLACEHOLDER`n-----END OPENSSH PRIVATE KEY-----`n"
        'credentials.json'         = '{"placeholder": true}'
        '.npmrc'                   = "//registry.example.com/:_authToken=PLACEHOLDER`n"
    }
    foreach ($name in $sensitiveSamples.Keys) {
        $path = Join-Path $Root $name
        Set-Content -LiteralPath $path -Value $sensitiveSamples[$name] -NoNewline -Encoding UTF8
    }
    $sshDir = Join-Path $Root '.ssh'
    [void][System.IO.Directory]::CreateDirectory($sshDir)
    Set-Content -LiteralPath (Join-Path $sshDir 'known_hosts') -Value "placeholder`n" -NoNewline -Encoding UTF8
}

$stopwatch.Stop()
Write-Host ("Done: {0} files under {1} directories in {2:N1}s ({3}) at {4}" -f `
    $FileCount, $directories.Count, $stopwatch.Elapsed.TotalSeconds, `
    $(if ($IncludeSensitiveSamples) { 'with sensitive samples' } else { 'no sensitive samples' }), $Root)
