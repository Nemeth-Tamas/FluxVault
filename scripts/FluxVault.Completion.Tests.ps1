param([string]$Executable)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'FluxVault.Completion.ps1')

function Complete([string]$Line, [int]$Cursor = -1) {
    if ($Cursor -lt 0) { $Cursor = $Line.Length }
    $tokens = $null; $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseInput($Line, [ref]$tokens, [ref]$parseErrors)
    $command = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.CommandAst] }, $true)
    $word = ''
    foreach ($element in @($command.CommandElements | Select-Object -Skip 1)) {
        if ($element.Extent.StartOffset -lt $Cursor -and $element.Extent.EndOffset -ge $Cursor) {
            $word = $Line.Substring($element.Extent.StartOffset, $Cursor - $element.Extent.StartOffset)
            break
        }
    }
    @(& $fluxVaultCompleter $word $command $Cursor | ForEach-Object { $_.CompletionText })
}
function Check([string]$Line, [string[]]$Expected, [string[]]$Excluded = @(), [int]$Cursor = -1) {
    $actual = @(Complete $Line $Cursor)
    foreach ($item in $Expected) { if ($actual -notcontains $item) { throw "Missing '$item' for '$Line': $($actual -join ', ')" } }
    foreach ($item in $Excluded) { if ($actual -contains $item) { throw "Invalid '$item' for '$Line'" } }
}

Check 'fv sc' @('scan') @('status','--usb')
Check 'fluxvault.exe gre' @('greaseweazle')
Check 'fv ' @('scan','acquire','finalize','completions')
Check 'fv greaseweazle c' @('capture','compare','consensus') @('scan')
Check 'fv scan --no' @('--no-verify') @('--profile')
Check 'fv scan --profile ibm.' @('ibm.1440','ibm.720') @('auto')
Check 'fv scan --profile ' @('auto','ibm.1440','ibm.720') @('--project')
Check 'fv greaseweazle capture 1 --profile ' @('ibm.1440','ibm.720') @('auto')
Check 'fv greaseweazle decode 1 --profile ' @('ibm.1440','ibm.720') @('auto')
Check 'fv report export --language ' @('hu','en') @('scan')
Check 'fv scan --gw-drive ' @('A','B') @('A:','B:')
Check 'fv scan --color n' @('never') @('always')
Check 'fv scan --so' @('--sound','--source-write-protected')
Check 'fv scan --sound ' @('on','off') @('always','auto','--project')
Check 'fv scan --usb --' @('--sound')
Check 'fv scan --double --' @('--sound')
Check 'fv production ' @('start','resume','status','queue','benchmark')
Check 'fv production start --' @('--last-disk','--destination','--no-verify','--double') @('--usb','--drive','--acquisition-only','--processing-mode','--plan')
Check 'fv production start --double --' @('--drive','--gw-drive','--write-blocker-verified','--destination') @('--no-verify','--profile','--count')
Check 'fv production resume --' @('--no-verify','--sound','--write-blocker-verified') @('--double','--last-disk','--acquisition-only')
Check 'fv start --sound o' @('on','off')
Check 'fv scan --double --plan --' @() @('--sound')
Check 'fv acquire --' @() @('--sound')
Check 'fv greaseweazle recover 7 --' @() @('--sound')
Check 'fv scan --capture-storage ' @('packed','raw')
Check 'fv scan --processing-mode ' @('background','tail')
Check 'fv scan --conversion-workers 1' @('1','12','16') @('2','--project')
Check 'fv finalize ' @('status','resume','--destination','--allow-attention')
Check 'fv finalize resume --' @('--allow-attention','--json') @('--destination','--conversion-workers')
Check 'fv finalize status --' @('--json') @('--allow-attention','--destination')
Check 'fv disk show 7 --' @('--details') @('--profile')
Check 'fv project ' @('show','import')
Check 'fv project import --' @('--source','--destination','--plan','--json') @('--project','--no-verify','--drive')
Check 'fv project import --source ''C:\old archive.zip'' --' @('--destination','--plan') @('--source','--project')
Check 'fv recovery se' @('sector') @('extract')
Check 'fv recovery sector 7 --' @('--lba','--sectors','--attempt') @('--capture-attempt','--profile')
Check 'fv recovery sector 7 --sectors ' @('1','8') @('0','9','--project')
Check 'fv recovery sector 7 --lba ' @() @('0','1','--project')
Check 'fv recovery compare 7 --' @() @('--lba','--sectors','--attempt')
Check 'fv disk next ' @('--json') @('show','next','list')
Check 'fv audit ' @('--json') @('scan','greaseweazle')
Check 'fv --project ''C:\A folder'' rec' @('recovery') @('--retries')
Check 'fv --project $unknown scan --profile a' @('auto')
Check 'fv --project ''--double'' scan --' @('--profile','--no-verify','--usb')
Check 'fv scan --json --' @('--last-disk') @('--json')
Check 'fv tools set ' @('sevenzip','libreoffice','greaseweazle')
Check 'fv scan --double --' @('--gw-drive','--plan','--last-disk') @('--no-verify','--profile','--usb')
Check 'fv scan --double --plan --' @('--last-disk') @('--conversion-workers','--color','--acquisition-only')
Check 'fv scan --usb --' @('--drive','--retries','--count') @('--double','--profile','--no-verify','--last-disk')
Check 'fv start --drive A: --' @('--retries') @('--profile','--no-verify')
Check 'fv greaseweazle recover 7 --' @('--profile','--policy') @('--conversion-workers')
Check 'fv greaseweazle preview --' @('--json') @('--project')
Check 'fv completions ' @('powershell') @('--project','--json')
Check 'fv completions powershell ' @() @('--project','powershell','--json')
Check 'fv sc --project ignored' @('scan') @('status') 5
Check 'fv scan --profile a --project ignored' @('auto') @('--project') 19

# Parsing a subexpression must never execute it while completing a later flag.
$script:completionExecuted = $false
Check 'fv --project $($script:completionExecuted = $true) scan --no' @('--no-verify')
if ($script:completionExecuted) { throw 'Completion evaluated input' }
if ((Complete 'fv scan --destination A: --last-disk ').Count -ne 0) { throw 'Disk labels must not be guessed' }

if ($Executable) {
    # Exercise the actual PowerShell engine, including the full-path .exe spelling.
    $line = "& '$Executable' scan --no"
    $native = [System.Management.Automation.CommandCompletion]::CompleteInput($line, $line.Length, $null)
    if (@($native.CompletionMatches.CompletionText) -notcontains '--no-verify') { throw 'Native full-path completion registration failed' }
}
Write-Output 'PowerShell completion checks passed.'
