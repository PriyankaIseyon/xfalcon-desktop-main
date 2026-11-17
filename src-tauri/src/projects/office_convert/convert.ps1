# Turns one Office file into a PDF with the Microsoft Office program that is
# installed, or with WPS Office when the app names it (XF_PROGID, XF_PROCESS).
# The paths and the file kind arrive in environment variables, never in this
# text. Microsoft Office runs with macros forced off; the file is opened
# read-only, with no prompts. Exit codes: 0 done, 10 the program is not
# installed, 12 the program could not open the file, 1 any other failure. The
# same code is written to XF_DONEFILE as soon as it is known, before the
# program is closed, because closing it can take a minute on some machines.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$in = $env:XF_IN
$out = $env:XF_OUT
$kind = $env:XF_KIND
$pidFile = $env:XF_PIDFILE
$doneFile = $env:XF_DONEFILE
# Set only for a program other than Microsoft Office that has the same object
# model (WPS Office): its COM class and its process name.
$otherProgId = $env:XF_PROGID
$otherProcess = $env:XF_PROCESS

$programs = @{
    word       = @('Word.Application', 'WINWORD')
    powerpoint = @('PowerPoint.Application', 'POWERPNT')
    excel      = @('Excel.Application', 'EXCEL')
}
if (-not $programs.ContainsKey($kind)) { exit 1 }
$progId = $programs[$kind][0]
$processName = $programs[$kind][1]
$microsoft = -not $otherProgId
if (-not $microsoft) {
    if (-not $otherProcess) { exit 1 }
    $progId = $otherProgId
    $processName = $otherProcess
}

function Get-ProgramIds {
    @(Get-Process -Name $processName -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
}

$code = 1
$doNotSave = 0   # wdDoNotSaveChanges; Word's Close and Quit take it by reference
$app = $null
$item = $null
$started = $false
$previousSecurity = $null
$before = Get-ProgramIds
try {
    try { $app = New-Object -ComObject $progId } catch { $code = 10; throw }

    # Only a program this script started is hidden, reported for cleanup and
    # closed afterwards. One the person already had open is left as it was.
    $mine = @(Get-ProgramIds | Where-Object { $before -notcontains $_ })
    if ($mine.Count -gt 0) {
        $started = $true
        Set-Content -LiteralPath $pidFile -Value $mine[0] -Encoding ASCII
    }

    # Microsoft Office must take the macro switch; another program is only given
    # formats that cannot hold a macro (the app checks that), and may lack it.
    if ($microsoft) {
        try { $previousSecurity = $app.AutomationSecurity } catch { }
        $app.AutomationSecurity = 3   # msoAutomationSecurityForceDisable: no macros
    }
    if ($started) {
        try {
            if ($kind -eq 'word') { $app.Visible = $false; $app.DisplayAlerts = 0 }
            if ($kind -eq 'excel') { $app.Visible = $false; $app.DisplayAlerts = $false }
            if ($kind -eq 'powerpoint') { $app.DisplayAlerts = 1 }
        } catch { if ($microsoft) { throw } }
    }

    $missing = [Type]::Missing
    switch ($kind) {
        'word' {
            # Read-only, not added to recent files, no conversion prompt, a
            # throwaway password so a protected file fails instead of asking,
            # and no window.
            try {
                $item = $app.Documents.Open($in, $false, $true, $false, 'xfalcon-no-password', $missing, $missing, $missing, $missing, $missing, $missing, $false)
            } catch { $code = 12; throw }
            $item.ExportAsFixedFormat($out, 17)   # wdExportFormatPDF
        }
        'powerpoint' {
            try {
                $item = $app.Presentations.Open($in, -1, -1, 0)   # read-only, untitled, no window
            } catch { $code = 12; throw }
            $item.SaveAs($out, 32)   # ppSaveAsPDF
        }
        'excel' {
            try {
                $item = $app.Workbooks.Open($in, 0, $true, $missing, 'xfalcon-no-password')   # never update links, read-only
            } catch { $code = 12; throw }
            $item.ExportAsFixedFormat(0, $out)   # xlTypePDF
        }
    }
    $code = 0
} catch {
    # $code already says why; an unlabelled failure stays 1.
} finally {
    # Report the result first. The PDF is already complete; closing the workbook
    # and quitting the program below may be slow, and the app need not wait.
    # Written under another name and renamed, so it is never read half-written.
    if ($doneFile) {
        try {
            $reported = $doneFile + '.tmp'
            Set-Content -LiteralPath $reported -Value $code -Encoding ASCII
            Move-Item -LiteralPath $reported -Destination $doneFile -Force
        } catch { }
    }
    try {
        if ($null -ne $item) {
            if ($kind -eq 'word') { $item.Close([ref]$doNotSave) }
            elseif ($kind -eq 'excel') { $item.Close($false) }
            else { $item.Close() }
        }
    } catch { }
    try { if ($null -ne $previousSecurity) { $app.AutomationSecurity = $previousSecurity } } catch { }
    try {
        if ($started -and $null -ne $app) {
            if ($kind -eq 'word') { $app.Quit([ref]$doNotSave) } else { $app.Quit() }
        }
    } catch { }
    # The COM references are deliberately not released by hand: releasing the
    # Excel object blocks until Excel has finished shutting down, about a minute
    # on some machines. Ending this process releases them.
}
exit $code
