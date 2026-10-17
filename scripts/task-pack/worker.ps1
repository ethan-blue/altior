#requires -Version 7.2
# One task-pack task inside its own worktree: execute -> verify -> commit -> review,
# with bounded fix rounds. Only writes inside the worktree and the task run directory.
param([Parameter(Mandatory)][string]$ContextJson)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3.0

$ctx = $ContextJson | ConvertFrom-Json
$task = $ctx.task
$wt = $ctx.worktree
$runDir = $ctx.runDir
$utf8 = [System.Text.UTF8Encoding]::new($false)
New-Item -ItemType Directory -Force -Path $runDir | Out-Null

$result = [ordered]@{
    id         = $task.id
    status     = 'RUNNING'
    branch     = $ctx.branch
    worktree   = $wt
    startPoint = $ctx.startPoint
    rounds     = 0
    costUsd    = 0.0
    commits    = @()
    sessions   = @()
    note       = ''
}

function Save-Result {
    [IO.File]::WriteAllText((Join-Path $runDir 'result.json'), ($result | ConvertTo-Json -Depth 5), $utf8)
}

function Complete-Task([string]$Status, [string]$Note) {
    $result.status = $Status
    $result.note = $Note
    Save-Result
    exit 0
}

function Get-Tail([string]$Text, [int]$Chars = 4000) {
    if (-not $Text) { return '' }
    if ($Text.Length -le $Chars) { return $Text }
    return '...' + $Text.Substring($Text.Length - $Chars)
}

function Invoke-Process {
    param(
        [string]$File,
        [string[]]$Arguments,
        [string]$StdinText,
        [int]$TimeoutMinutes
    )
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $File
    foreach ($a in $Arguments) { $psi.ArgumentList.Add($a) }
    $psi.WorkingDirectory = $wt
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardInputEncoding = $utf8
    $psi.StandardOutputEncoding = $utf8
    $psi.StandardErrorEncoding = $utf8
    # Per-slot target dir and visual-gate port: parallel tasks never share cargo's
    # build-directory lock or the fixed Vite port of the visual gate.
    $psi.Environment['CARGO_TARGET_DIR'] = $ctx.cargoTargetDir
    $psi.Environment['ALTIOR_VISUAL_PORT'] = [string]$ctx.visualPort
    $p = [System.Diagnostics.Process]::Start($psi)
    $stdout = $p.StandardOutput.ReadToEndAsync()
    $stderr = $p.StandardError.ReadToEndAsync()
    if ($StdinText) { $p.StandardInput.Write($StdinText) }
    $p.StandardInput.Close()
    $timedOut = -not $p.WaitForExit($TimeoutMinutes * 60 * 1000)
    if ($timedOut) {
        $p.Kill($true)
        $p.WaitForExit()
    }
    [pscustomobject]@{
        ExitCode = if ($timedOut) { -1 } else { $p.ExitCode }
        TimedOut = $timedOut
        Stdout   = $stdout.GetAwaiter().GetResult()
        Stderr   = $stderr.GetAwaiter().GetResult()
    }
}

function Invoke-Shell([string]$Command, [string]$LogName, [int]$TimeoutMinutes) {
    $shell = if ($IsWindows) { @('cmd.exe', '/d', '/s', '/c') } else { @('/bin/sh', '-c') }
    $r = Invoke-Process -File $shell[0] -Arguments (@($shell[1..($shell.Count - 1)]) + $Command) -TimeoutMinutes $TimeoutMinutes
    $log = "`$ $Command`n$($r.Stdout)`n$($r.Stderr)"
    if ($r.TimedOut) { $log += "`n[task-pack] killed after $TimeoutMinutes minutes" }
    [IO.File]::WriteAllText((Join-Path $runDir $LogName), $log, $utf8)
    [pscustomobject]@{ ExitCode = $r.ExitCode; Tail = (Get-Tail $log 3000) }
}

function Invoke-Agent {
    param(
        [string]$Role,
        [string]$Model,
        [string]$Prompt,
        [string]$SettingsPath,
        [int]$MaxTurns,
        [int]$TimeoutMinutes
    )
    [IO.File]::WriteAllText((Join-Path $runDir "$Role.prompt.md"), $Prompt, $utf8)
    $claudeArgs = @(
        '-p',
        '--model', $Model,
        '--output-format', 'json',
        '--max-turns', "$MaxTurns",
        '--permission-mode', 'acceptEdits',
        '--settings', $SettingsPath
    )
    $r = Invoke-Process -File $ctx.claudePath -Arguments $claudeArgs -StdinText $Prompt -TimeoutMinutes $TimeoutMinutes
    [IO.File]::WriteAllText((Join-Path $runDir "$Role.json"), $r.Stdout, $utf8)
    if ($r.Stderr) { [IO.File]::WriteAllText((Join-Path $runDir "$Role.stderr.log"), $r.Stderr, $utf8) }
    $json = $null
    try { $json = $r.Stdout | ConvertFrom-Json } catch { $json = $null }
    if ($null -eq $json -or -not ($json.PSObject.Properties.Name -contains 'result')) {
        return [pscustomobject]@{ Ok = $false; Text = (Get-Tail "$($r.Stdout)`n$($r.Stderr)" 2000); Subtype = 'no_json' }
    }
    if ($json.PSObject.Properties.Name -contains 'total_cost_usd') { $result.costUsd += [double]$json.total_cost_usd }
    if ($json.PSObject.Properties.Name -contains 'session_id') { $result.sessions += "$Role=$($json.session_id)" }
    Save-Result
    [pscustomobject]@{
        Ok      = (-not $r.TimedOut)
        Text    = [string]$json.result
        Subtype = if ($r.TimedOut) { 'timeout' } else { [string]$json.subtype }
    }
}

function Get-Marker([string]$Text, [string]$Name, [string]$Values) {
    $found = [regex]::Matches($Text, "(?im)^\W*$Name\W*[:：]\s*\W*($Values)\b")
    if ($found.Count -eq 0) { return $null }
    return $found[$found.Count - 1].Groups[1].Value.ToUpperInvariant()
}

function Expand-Template([string]$Template, [hashtable]$Extra) {
    $values = @{
        TASK_ID       = $task.id
        TASK_TITLE    = $task.title
        TASK_SPEC     = $ctx.spec
        PACK          = $ctx.pack
        EVIDENCE_PATH = $ctx.evidencePath
        START_POINT   = $ctx.startPoint
        RULES         = $ctx.templates.rules
        VERIFY        = (($task.verify | ForEach-Object { "- ``$_``" }) -join "`n")
    }
    foreach ($k in $Extra.Keys) { $values[$k] = $Extra[$k] }
    # RULES first: the shared rules text carries placeholders of its own.
    $out = $Template.Replace('{{RULES}}', [string]$values['RULES'])
    foreach ($k in $values.Keys) { $out = $out.Replace("{{$k}}", [string]$values[$k]) }
    return $out
}

function Invoke-Git([string[]]$GitArgs) {
    $out = & git -C $wt @GitArgs 2>&1
    if ($LASTEXITCODE -ne 0) { throw "git $($GitArgs -join ' ') failed: $out" }
    return $out
}

try {
    Save-Result

    if (-not (Test-Path -LiteralPath (Join-Path $wt 'apps/desktop/node_modules'))) {
        $npm = Invoke-Shell 'npm ci --prefix apps/desktop --prefer-offline --no-audit --no-fund' 'setup-npm-ci.log' 20
        if ($npm.ExitCode -ne 0) { Complete-Task 'SETUP_FAILED' "npm ci failed:`n$($npm.Tail)" }
    }

    $feedback = ''
    for ($round = 0; $round -le [int]$ctx.fixRounds; $round++) {
        $result.rounds = $round + 1
        Save-Result

        $template = if ($round -eq 0) { $ctx.templates.executor } else { $ctx.templates.fix }
        $prompt = Expand-Template $template @{ FEEDBACK = $feedback }
        $exec = Invoke-Agent -Role "executor-r$round" -Model $task.executor -Prompt $prompt `
            -SettingsPath $ctx.executorSettings -MaxTurns ([int]$ctx.maxTurns) -TimeoutMinutes ([int]$ctx.agentTimeoutMinutes)
        if ($exec.Subtype -eq 'no_json') { Complete-Task 'AGENT_ERROR' "Executor produced no JSON result:`n$($exec.Text)" }

        $declared = Get-Marker $exec.Text 'STATUS' 'DONE|PARTIAL|BLOCKED'
        if ($declared -eq 'BLOCKED') { Complete-Task 'BLOCKED' (Get-Tail $exec.Text 1500) }

        $failures = @()
        if (-not $exec.Ok) { $failures += "上一轮执行被中止（$($exec.Subtype)），工作可能不完整。" }
        if ($declared -eq 'PARTIAL') { $failures += "你上一轮报告 STATUS: PARTIAL，剩余工作：`n$(Get-Tail $exec.Text 1500)" }
        if (-not (Test-Path -LiteralPath (Join-Path $wt $ctx.evidencePath))) {
            $failures += "缺少证据文件 $($ctx.evidencePath)（必须写 work brief 与真实验证记录）。"
        }
        $verifyLines = @()
        $i = 0
        foreach ($command in $task.verify) {
            $i++
            $v = Invoke-Shell $command "verify-r$round-$i.log" ([int]$ctx.verifyTimeoutMinutes)
            $verifyLines += "- ``$command`` -> exit $($v.ExitCode)"
            if ($v.ExitCode -ne 0) { $failures += "验证命令失败（exit $($v.ExitCode)）：$command`n``````text`n$($v.Tail)`n``````" }
        }
        $verifyReport = $verifyLines -join "`n"

        if ($failures.Count -gt 0) {
            $feedback = "## 由执行器脚本检测到的问题`n`n" + ($failures -join "`n`n")
            continue
        }

        $pending = Invoke-Git @('status', '--porcelain')
        if ($pending) {
            Invoke-Git @('add', '-A') | Out-Null
            $subject = if ($round -eq 0) { $task.commit } else { "$($task.commit) (review round $($round + 1))" }
            $body = "Task: $($task.id) - $($task.title)`nPack: $($ctx.pack)`nExecutor: $($task.executor); reviewer: $($task.reviewer)"
            Invoke-Git @('-c', 'gc.auto=0', 'commit', '-q', '-m', $subject, '-m', $body) | Out-Null
            $result.commits += [string](Invoke-Git @('rev-parse', 'HEAD'))
            Save-Result
        }
        if ($result.commits.Count -eq 0) { Complete-Task 'NO_CHANGES' 'Executor finished without changing any file.' }

        if ($ctx.skipReview) { Complete-Task 'PASSED' 'review skipped (-SkipReview)' }

        $reviewPrompt = Expand-Template $ctx.templates.reviewer @{ VERIFY_RESULTS = $verifyReport }
        $review = Invoke-Agent -Role "reviewer-r$round" -Model $task.reviewer -Prompt $reviewPrompt `
            -SettingsPath $ctx.reviewerSettings -MaxTurns 60 -TimeoutMinutes 30
        [IO.File]::WriteAllText((Join-Path $runDir "review-r$round.md"), $review.Text, $utf8)
        $verdict = Get-Marker $review.Text 'VERDICT' 'PASS|FAIL'
        if ($verdict -eq 'PASS') { Complete-Task 'PASSED' '' }
        $feedback = if ($verdict) {
            "## 独立审查结论 FAIL`n`n$(Get-Tail $review.Text 6000)"
        } else {
            "## 审查未给出 VERDICT（按 FAIL 处理）`n`n$(Get-Tail $review.Text 3000)"
        }
    }
    Complete-Task 'FAILED' (Get-Tail $feedback 1500)
}
catch {
    Complete-Task 'RUNNER_ERROR' "$($_.Exception.Message)`n$($_.ScriptStackTrace)"
}
