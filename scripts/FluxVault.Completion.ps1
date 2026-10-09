# Native completion for PowerShell 7+. Static data only: no CLI execution,
# media probes, project lookups, profile writes or filesystem enumeration.
# Load into the current shell by dot-sourcing this trusted repository script.
if ($PSVersionTable.PSVersion.Major -lt 7) {
    Write-Warning 'FluxVault native Tab completion requires PowerShell 7 or newer.'
    return
}

$fluxVaultCompleter = {
    param($wordToComplete, $commandAst, $cursorPosition)

    $children = @{
        '' = @('init','status','start','stop','scan','acquire','disk','project','drive','tools','greaseweazle','diagnose','extract','recovery','conversion','files','audit','report','process','processing','storage','benchmark','production','finalize','package','run','completions','help')
        'disk' = @('list','show','select','next')
        'project' = @('show')
        'drive' = @('list','probe')
        'tools' = @('check','show','set','clear')
        'greaseweazle' = @('preview','info','capture','decode','identify','status','diagnose','compare','consensus','plan','recover','scan')
        'extract' = @('all','disk')
        'recovery' = @('plan','compare','backup','queue','composite','fat','extract','documents','import')
        'conversion' = @('plan','run','issues','retry')
        'files' = @('manifest')
        'report' = @('export')
        'processing' = @('status','resume')
        'storage' = @('benchmark','pack','resume')
        'benchmark' = @('report','compare')
        'production' = @('status','queue','benchmark')
        'finalize' = @('status','resume')
        'package' = @('build')
        'run' = @('status')
        'completions' = @('powershell')
        'tools set' = @('sevenzip','libreoffice','greaseweazle')
        'tools clear' = @('sevenzip','libreoffice','greaseweazle')
    }
    # Every real leaf needs an empty child list; don't fall back to its parent's
    # subcommands after a complete command or a numeric disk argument.
    foreach ($parent in @($children.Keys)) {
        foreach ($child in @($children[$parent])) {
            $leaf = if ($parent) { "$parent $child" } else { $child }
            if (-not $children.ContainsKey($leaf)) { $children[$leaf] = @() }
        }
    }
    $scan = @('--last-disk','--count','--no-verify','--conversion-workers','--gw-drive','--profile','--profile-map','--policy','--capture-storage','--processing-mode','--acquisition-only','--source-write-protected','--color','--usb','--double','--plan','--drive','--retries','--write-blocker-verified')
    $options = @{
        'scan' = $scan
        'start' = $scan
        'greaseweazle scan' = @('--last-disk','--count','--no-verify','--conversion-workers','--gw-drive','--profile','--profile-map','--policy','--capture-storage','--processing-mode','--acquisition-only','--source-write-protected','--color')
        'disk show' = @('--details')
        'drive probe' = @('--drive')
        'acquire' = @('--drive','--disk','--retries','--write-blocker-verified')
        'greaseweazle capture' = @('--gw-drive','--profile','--revs','--source-write-protected')
        'greaseweazle decode' = @('--capture-attempt','--profile')
        'greaseweazle identify' = @('--capture-attempt')
        'greaseweazle recover' = @('--gw-drive','--profile','--policy','--source-write-protected','--acquisition-only')
        'recovery extract' = @('--include-deleted')
        'recovery import' = @('--source','--dmde-log')
        'conversion run' = @('--conversion-workers')
        'conversion retry' = @('--conversion-workers')
        'report export' = @('--language')
        'process' = @('--conversion-workers')
        'processing resume' = @('--conversion-workers')
        'storage pack' = @('--capture-attempt','--retire-raw')
        'benchmark compare' = @('--baseline','--include-deleted')
        'finalize' = @('--destination','--conversion-workers','--allow-attention')
        'finalize resume' = @('--allow-attention')
        'package build' = @('--destination')
    }
    $values = @{
        '--profile' = @('auto','ibm.1440','ibm.720')
        '--gw-drive' = @('A','B')
        '--language' = @('hu','en')
        '--color' = @('auto','always','never')
        '--capture-storage' = @('packed','raw')
        '--processing-mode' = @('background','tail')
        '--conversion-workers' = @('1','2','4','8','12','16')
        '--retries' = @(0..10 | ForEach-Object { [string]$_ })
        '--revs' = @(1..10 | ForEach-Object { [string]$_ })
    }
    $takesValue = @('--project','--destination','--drive','--disk','--retries','--count','--last-disk','--source','--baseline','--dmde-log','--conversion-workers','--gw-drive','--profile','--capture-storage','--processing-mode','--color','--revs','--capture-attempt','--policy','--profile-map','--language')

    # Only literal AST text before the cursor; never evaluate a variable,
    # subexpression, command substitution, or any user-supplied argument.
    $words = @()
    foreach ($element in @($commandAst.CommandElements | Select-Object -Skip 1)) {
        if ($element.Extent.StartOffset -ge $cursorPosition) { break }
        if ($element.Extent.EndOffset -ge $cursorPosition -and $wordToComplete) { break }
        if ($element -is [System.Management.Automation.Language.StringConstantExpressionAst]) {
            $words += $element.Value
        } else {
            $words += $element.Extent.Text
        }
    }
    $positionals = @()
    $flags = @()
    $expectValue = $null
    foreach ($word in $words) {
        if ($expectValue) { $expectValue = $null; continue }
        if ($word.StartsWith('--', [StringComparison]::Ordinal)) {
            $flags += $word
            if ($takesValue -contains $word) { $expectValue = $word }
        } else { $positionals += $word }
    }
    $context = $positionals -join ' '
    # Numeric disk labels/paths are arguments, not new command branches.
    while ($context -and -not $children.ContainsKey($context) -and -not $options.ContainsKey($context)) {
        $positionals = @($positionals | Select-Object -SkipLast 1)
        $context = $positionals -join ' '
    }
    $prefix = ([string]$wordToComplete).Trim([char[]](39,34))
    if ($context -in @('scan','start')) {
        if ($flags -contains '--double') {
            $options[$context] = @('--drive','--gw-drive','--last-disk','--write-blocker-verified','--conversion-workers','--acquisition-only','--color','--plan')
            if ($flags -contains '--plan') { $options[$context] = @('--drive','--gw-drive','--last-disk') }
        } elseif ($flags -contains '--usb' -or $flags -contains '--drive' -or $flags -contains '--write-blocker-verified') {
            $options[$context] = @('--drive','--count','--retries','--write-blocker-verified')
        }
    }
    if ($expectValue) {
        $candidates = @($values[$expectValue])
        # Fixed-only capture/decode must not suggest the automatic profile.
        if ($expectValue -eq '--profile' -and $context -in @('greaseweazle capture','greaseweazle decode')) {
            $candidates = @('ibm.1440','ibm.720')
        }
    } elseif ($context -eq 'completions') {
        $candidates = @('powershell')
    } elseif ($context -eq 'completions powershell') {
        $candidates = @()
    } else {
        $candidates = @()
        if (-not $prefix.StartsWith('-')) { $candidates += @($children[$context]) }
        if (-not $prefix -or $prefix.StartsWith('-')) {
            $globalOptions = if ($context -eq 'greaseweazle preview') { @('--json','--help') } else { @('--project','--json','--help') }
            $candidates += $globalOptions + @($options[$context])
            $candidates = @($candidates | Where-Object { $flags -notcontains $_ })
        }
    }
    foreach ($candidate in @($candidates | Where-Object { $_ } | Sort-Object -Unique)) {
        if ($candidate.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
            $type = if ($candidate.StartsWith('--')) { 'ParameterName' } else { 'ParameterValue' }
            [System.Management.Automation.CompletionResult]::new($candidate, $candidate, $type, "FluxVault: $candidate")
        }
    }
}

Register-ArgumentCompleter -Native -CommandName 'fv','fv.exe','fluxvault','fluxvault.exe' -ScriptBlock $fluxVaultCompleter
