#requires -Version 7.2
<#
.SYNOPSIS
Runs a docs/reviews task pack with headless Claude Code agents, one git worktree per task.

.DESCRIPTION
Waves run in order. Inside a wave, tasks run in parallel, each on its own branch cut
from the integration branch tip. A task merges into the integration branch only when
its verify commands pass and an independent reviewer answers VERDICT: PASS; after each
wave the full quality gate runs on the integration branch. Your own working tree is
never modified: everything happens in sibling worktrees and agent/* branches.

.EXAMPLE
pwsh -NoProfile -File scripts/task-pack/run.ps1 -DryRun
pwsh -NoProfile -File scripts/task-pack/run.ps1 -Wave 1
pwsh -NoProfile -File scripts/task-pack/run.ps1 -Task C03,C07 -Parallel 2
#>
[CmdletBinding()]
param(
    [string]$Pack = 'docs/reviews/2026-09-26',
    [string[]]$Task,
    [int[]]$Wave,
    [string]$Base = 'HEAD',
    [ValidateRange(1, 8)][int]$Parallel = 3,
    [ValidateRange(0, 3)][int]$FixRounds = 1,
    [int]$MaxTurns = 150,
    [int]$AgentTimeoutMinutes = 90,
    [int]$VerifyTimeoutMinutes = 45,
    [string]$WorktreeRoot,
    [switch]$SkipReview,
    [switch]$SkipWaveGate,
    [switch]$Fresh,
    [switch]$AllowDirty,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3.0
$utf8 = [System.Text.UTF8Encoding]::new($false)

function Invoke-Git {
    param([string]$Dir, [string[]]$GitArgs, [switch]$AllowFailure)
    $out = & git -C $Dir @GitArgs 2>&1
    if ($LASTEXITCODE -ne 0 -and -not $AllowFailure) { throw "git $($GitArgs -join ' ') failed: $out" }
    return $out
}

$repo = (Invoke-Git -Dir $PWD.Path -GitArgs @('rev-parse', '--show-toplevel')).Trim()
$packRel = $Pack.Replace('\', '/').TrimEnd('/')
$packDir = Join-Path $repo $packRel
$packName = Split-Path $packRel -Leaf
$manifest = Get-Content -LiteralPath (Join-Path $packDir 'tasks.json') -Raw | ConvertFrom-Json
$tasksMd = Get-Content -LiteralPath (Join-Path $packDir 'TASKS.md') -Raw
$templates = [ordered]@{}
foreach ($name in 'rules', 'executor', 'fix', 'reviewer') {
    $templates[$name] = Get-Content -LiteralPath (Join-Path $packDir "prompts/$name.md") -Raw
}

function Get-TaskSpec([string]$Id) {
    $m = [regex]::Match($tasksMd, "(?ms)^### $([regex]::Escape($Id))\b.*?(?=^#{2,3} |\z)")
    if ($m.Success) { return $m.Value.Trim() }
    return $null
}

# ---- manifest validation -------------------------------------------------------------
$allTasks = @($manifest.tasks)
$byId = @{}
$problems = [System.Collections.Generic.List[string]]::new()
foreach ($t in $allTasks) {
    if ($byId.ContainsKey($t.id)) { $problems.Add("duplicate id $($t.id)") }
    $byId[$t.id] = $t
}
$commitPattern = '^(feat|fix|docs|test|refactor|perf|build|ci|chore|style)(\([a-z0-9-]+\))?!?: \S.{0,90}$'
foreach ($t in $allTasks) {
    if ($t.executor -notin 'haiku', 'sonnet', 'opus') { $problems.Add("$($t.id): executor must be haiku|sonnet|opus") }
    if ($t.reviewer -notin 'haiku', 'sonnet', 'opus') { $problems.Add("$($t.id): reviewer must be haiku|sonnet|opus") }
    if ($t.commit -notmatch $commitPattern) { $problems.Add("$($t.id): commit subject is not Conventional Commits: $($t.commit)") }
    if (-not $t.manual -and @($t.verify).Count -eq 0) { $problems.Add("$($t.id): automated task needs at least one verify command") }
    foreach ($cmd in @($t.verify)) { if ($cmd -match '"') { $problems.Add("$($t.id): verify commands must not contain double quotes") } }
    if (-not (Get-TaskSpec $t.id)) { $problems.Add("$($t.id): no '### $($t.id)' section in TASKS.md") }
    foreach ($dep in @($t.blockedBy)) {
        if (-not $byId.ContainsKey($dep)) { $problems.Add("$($t.id): unknown dependency $dep") }
        elseif ($byId[$dep].wave -ge $t.wave) { $problems.Add("$($t.id): dependency $dep must be in an earlier wave") }
        elseif ($byId[$dep].manual -and -not $t.manual) { $problems.Add("$($t.id): automated task depends on manual task $dep") }
    }
}
if ($problems.Count) { throw "tasks.json is invalid:`n - " + ($problems -join "`n - ") }

# ---- state ---------------------------------------------------------------------------
$runRoot = Join-Path $repo "target/task-pack/$packName"
$statePath = Join-Path $runRoot 'state.json'
New-Item -ItemType Directory -Force -Path $runRoot | Out-Null
$state = if (Test-Path -LiteralPath $statePath) { Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json -AsHashtable } else { @{} }
function Save-State { [IO.File]::WriteAllText($statePath, ($state | ConvertTo-Json -Depth 6), $utf8) }
function Get-Status([string]$Id) { if ($state.ContainsKey($Id)) { return $state[$Id].status } return 'TODO' }

$selected = @($allTasks | Where-Object {
        -not $_.manual -and (
            ($Task -and $Task -contains $_.id) -or
            (-not $Task -and (-not $Wave -or $Wave -contains $_.wave))
        )
    } | Sort-Object wave, id)
$manualTasks = @($allTasks | Where-Object manual)
if ($Task) {
    $unknown = @($Task | Where-Object { -not $byId.ContainsKey($_) })
    if ($unknown) { throw "Unknown task id(s): $($unknown -join ', ')" }
}

$wtRoot = if ($WorktreeRoot) { $WorktreeRoot } else { Join-Path (Split-Path $repo -Parent) "$(Split-Path $repo -Leaf)-agents/$packName" }
$integBranch = $manifest.integrationBranch

Write-Host "Task pack  : $packRel ($($allTasks.Count) tasks, $($manualTasks.Count) manual)"
Write-Host "Worktrees  : $wtRoot"
Write-Host "Integration: $integBranch"
$selected | ForEach-Object {
    [pscustomobject]@{
        Wave = $_.wave; Id = $_.id; Executor = $_.executor; Reviewer = $_.reviewer
        Deps = (@($_.blockedBy) -join ','); State = (Get-Status $_.id); Title = $_.title
    }
} | Format-Table -AutoSize | Out-String -Width 200 | Write-Host
if ($manualTasks) {
    Write-Host "Manual tasks (never run by agents, need you): $(($manualTasks | ForEach-Object id) -join ', ')"
}
if ($DryRun) { Write-Host 'Dry run: manifest valid, nothing executed.'; return }
if (-not $selected) { Write-Host 'Nothing selected.'; return }

# ---- preflight -----------------------------------------------------------------------
$claudeCmd = if ($IsWindows) { 'claude.cmd' } else { 'claude' }
$claudePath = (Get-Command $claudeCmd -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source
if (-not $claudePath) { throw "Claude Code CLI ($claudeCmd) was not found on PATH." }
$baseSha = (Invoke-Git -Dir $repo -GitArgs @('rev-parse', '--verify', "$Base^{commit}")).Trim()
Invoke-Git -Dir $repo -GitArgs @('cat-file', '-e', "${baseSha}:$packRel/TASKS.md") -AllowFailure | Out-Null
if ($LASTEXITCODE -ne 0) {
    throw "The task pack is not committed at $Base. Agents work from commits, so commit $packRel and scripts/task-pack first."
}
$dirty = Invoke-Git -Dir $repo -GitArgs @('status', '--porcelain')
if ($dirty -and -not $AllowDirty) {
    throw "Your working tree has uncommitted changes. Agents branch from $Base and will NOT see them. Commit first, or pass -AllowDirty if that is intended."
}

# ---- worktrees -----------------------------------------------------------------------
function Test-Branch([string]$Name) {
    Invoke-Git -Dir $repo -GitArgs @('rev-parse', '--verify', '--quiet', "refs/heads/$Name") -AllowFailure | Out-Null
    return $LASTEXITCODE -eq 0
}

function New-Worktree([string]$Path, [string]$Branch, [string]$StartPoint) {
    if (Test-Path -LiteralPath $Path) { return }
    New-Item -ItemType Directory -Force -Path (Split-Path $Path -Parent) | Out-Null
    if (Test-Branch $Branch) { Invoke-Git -Dir $repo -GitArgs @('worktree', 'add', $Path, $Branch) | Out-Null }
    else { Invoke-Git -Dir $repo -GitArgs @('worktree', 'add', '-b', $Branch, $Path, $StartPoint) | Out-Null }
}

$integ = Join-Path $wtRoot '_integration'
New-Worktree $integ $integBranch $baseSha

# ---- run -----------------------------------------------------------------------------
$worker = Join-Path $PSScriptRoot 'worker.ps1'
$checkpointRows = [System.Collections.Generic.List[string]]::new()
$stopReason = $null

foreach ($w in @($selected | ForEach-Object wave | Sort-Object -Unique)) {
    $ready = @()
    foreach ($t in @($selected | Where-Object wave -eq $w)) {
        if ((Get-Status $t.id) -eq 'MERGED') { continue }
        $missing = @(@($t.blockedBy) | Where-Object { (Get-Status $_) -ne 'MERGED' })
        if ($missing) {
            $state[$t.id] = @{ status = 'SKIPPED_DEP'; note = "waiting for $($missing -join ', ')" }
            continue
        }
        $ready += $t
    }
    Save-State
    if (-not $ready) { continue }

    $tip = (Invoke-Git -Dir $integ -GitArgs @('rev-parse', 'HEAD')).Trim()
    Write-Host "`n=== Wave $w : $($ready.Count) task(s) from $($tip.Substring(0, 10)) ==="

    $contexts = @{}
    foreach ($t in $ready) {
        $branch = "agent/$packName/$($t.id.ToLowerInvariant())"
        $path = Join-Path $wtRoot $t.id
        if ($Fresh -and (Test-Path -LiteralPath $path)) {
            Invoke-Git -Dir $repo -GitArgs @('worktree', 'remove', '--force', $path) | Out-Null
            Invoke-Git -Dir $repo -GitArgs @('branch', '-D', $branch) -AllowFailure | Out-Null
        }
        New-Worktree $path $branch $tip
        $startPoint = (Invoke-Git -Dir $repo -GitArgs @('merge-base', $branch, $tip)).Trim()
        $contexts[$t.id] = [ordered]@{
            task                 = $t
            spec                 = Get-TaskSpec $t.id
            pack                 = $packRel
            branch               = $branch
            worktree             = $path
            startPoint           = $startPoint
            evidencePath         = "$packRel/work/$($t.id).md"
            runDir               = Join-Path $runRoot $t.id
            templates            = $templates
            claudePath           = $claudePath
            executorSettings     = Join-Path $PSScriptRoot 'executor.settings.json'
            reviewerSettings     = Join-Path $PSScriptRoot 'reviewer.settings.json'
            fixRounds            = $FixRounds
            maxTurns             = $MaxTurns
            agentTimeoutMinutes  = $AgentTimeoutMinutes
            verifyTimeoutMinutes = $VerifyTimeoutMinutes
            skipReview           = [bool]$SkipReview
            cargoTargetDir       = $null
            visualPort           = $null
        }
    }

    $pending = [System.Collections.Generic.Queue[object]]::new()
    $ready | ForEach-Object { $pending.Enqueue($_) }
    $slots = [System.Collections.Generic.Queue[int]]::new()
    1..$Parallel | ForEach-Object { $slots.Enqueue($_) }
    $running = @{}
    $deadline = [TimeSpan]::FromMinutes(($FixRounds + 1) * ($AgentTimeoutMinutes + 30 + $VerifyTimeoutMinutes * 2) + 30)

    while ($pending.Count -or $running.Count) {
        while ($pending.Count -and $slots.Count) {
            $t = $pending.Dequeue()
            $slot = $slots.Dequeue()
            $c = $contexts[$t.id]
            $c.cargoTargetDir = Join-Path $repo "target/task-pack/cargo-slot-$slot"
            $c.visualPort = 4200 + $slot
            Remove-Item -LiteralPath (Join-Path $c.runDir 'result.json') -ErrorAction SilentlyContinue
            $job = Start-Job -FilePath $worker -ArgumentList ($c | ConvertTo-Json -Depth 8 -Compress)
            $running[$job.Id] = [pscustomobject]@{ Job = $job; Task = $t; Slot = $slot; Started = Get-Date }
            $state[$t.id] = @{ status = 'RUNNING'; branch = $c.branch; worktree = $c.worktree }
            Save-State
            Write-Host ("[{0:HH:mm}] start  {1} ({2} -> {3}) slot {4}" -f (Get-Date), $t.id, $t.executor, $t.reviewer, $slot)
        }
        Wait-Job -Job @($running.Values | ForEach-Object Job) -Any -Timeout 30 | Out-Null
        foreach ($r in @($running.Values)) {
            $expired = ((Get-Date) - $r.Started) -gt $deadline
            if ($r.Job.State -eq 'Running' -and -not $expired) { continue }
            if ($r.Job.State -eq 'Running') { Stop-Job -Job $r.Job }
            $c = $contexts[$r.Task.id]
            $jobLog = Receive-Job -Job $r.Job -ErrorAction SilentlyContinue *>&1 | Out-String
            [IO.File]::WriteAllText((Join-Path $c.runDir 'job.log'), $jobLog, $utf8)
            Remove-Job -Job $r.Job -Force
            $resultPath = Join-Path $c.runDir 'result.json'
            $res = if (Test-Path -LiteralPath $resultPath) { Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json -AsHashtable } else { @{ status = 'RUNNER_ERROR'; note = 'worker wrote no result' } }
            if ($expired) { $res.status = 'TIMEOUT'; $res.note = "stopped after $([int]$deadline.TotalMinutes) min" }
            $state[$r.Task.id] = $res
            Save-State
            $slots.Enqueue($r.Slot)
            $running.Remove($r.Job.Id)
            $cost = if ($res.ContainsKey('costUsd')) { '{0:N2}' -f $res.costUsd } else { '?' }
            Write-Host ("[{0:HH:mm}] finish {1} -> {2} (cost `${3})" -f (Get-Date), $r.Task.id, $res.status, $cost)
        }
    }

    # Merge passing branches in id order; a conflict leaves the branch for a human.
    $merged = 0
    foreach ($t in ($ready | Sort-Object id)) {
        $res = $state[$t.id]
        if ($res.status -eq 'PASSED') {
            Invoke-Git -Dir $integ -GitArgs @('merge', '--no-ff', '--no-edit', $res.branch) -AllowFailure | Out-Null
            if ($LASTEXITCODE -eq 0) { $res.status = 'MERGED'; $merged++ }
            else {
                Invoke-Git -Dir $integ -GitArgs @('merge', '--abort') -AllowFailure | Out-Null
                $res.status = 'CONFLICT'
                $res.note = "merge into $integBranch conflicted; resolve by hand or rerun with -Task $($t.id) -Fresh"
            }
        }
        $cost = if ($res.ContainsKey('costUsd')) { '{0:N2}' -f $res.costUsd } else { '-' }
        $note = ([string]$res.note -replace '\s+', ' ' -replace '\|', '/')
        if ($note.Length -gt 140) { $note = $note.Substring(0, 140) + '...' }
        $checkpointRows.Add("| $(Get-Date -Format 'yyyy-MM-dd HH:mm') | $w | $($t.id) | $($res.status) | ``$($res.branch)`` | $($t.executor)/$($t.reviewer) | $cost | $note |")
    }
    Save-State

    if ($checkpointRows.Count) {
        $cp = Join-Path $integ "$packRel/CHECKPOINT.md"
        $existing = [IO.File]::ReadAllText($cp)
        $prefix = if ($existing.EndsWith("`n")) { '' } else { "`n" }
        [IO.File]::AppendAllText($cp, $prefix + ($checkpointRows -join "`n") + "`n", $utf8)
        $checkpointRows.Clear()
        Invoke-Git -Dir $integ -GitArgs @('add', '--', "$packRel/CHECKPOINT.md") | Out-Null
        Invoke-Git -Dir $integ -GitArgs @('-c', 'gc.auto=0', 'commit', '-q', '-m', "docs(review): record task-pack wave $w results") | Out-Null
    }

    if ($merged -and -not $SkipWaveGate) {
        Write-Host "Wave $w : running scripts/quality-gate.ps1 on $integBranch ..."
        $gateLog = Join-Path $runRoot "wave-$w-gate.log"
        $gateTarget = Join-Path $repo 'target/task-pack/cargo-integration'
        $gateScript = "`$env:CARGO_TARGET_DIR = '$gateTarget'; `$env:ALTIOR_VISUAL_PORT = '4199'; Set-Location -LiteralPath '$integ'; " +
        "if (-not (Test-Path apps/desktop/node_modules)) { npm ci --prefix apps/desktop --prefer-offline --no-audit --no-fund; if (`$LASTEXITCODE) { exit `$LASTEXITCODE } }; " +
        "& ./scripts/quality-gate.ps1"
        & pwsh -NoProfile -ExecutionPolicy Bypass -Command $gateScript *> $gateLog
        if ($LASTEXITCODE -ne 0) {
            $stopReason = "quality gate failed after wave $w (see $gateLog); later waves were not started"
            break
        }
        Write-Host "Wave $w : quality gate passed."
    }
}

# ---- summary -------------------------------------------------------------------------
$total = 0.0
$rows = foreach ($t in $selected) {
    $s = if ($state.ContainsKey($t.id)) { $state[$t.id] } else { @{ status = 'NOT_RUN' } }
    if ($s.ContainsKey('costUsd')) { $total += [double]$s.costUsd }
    $note = [string]$s.note -replace '\s+', ' '
    [pscustomobject]@{
        Wave   = $t.wave
        Id     = $t.id
        Status = $s.status
        Cost   = if ($s.ContainsKey('costUsd')) { '{0:N2}' -f $s.costUsd } else { '' }
        Note   = if ($note.Length -gt 90) { $note.Substring(0, 90) + '...' } else { $note }
    }
}
$rows | Format-Table -AutoSize | Out-String -Width 220 | Write-Host
Write-Host ("Reported agent cost for this selection: `${0:N2}. Logs: {1}" -f $total, $runRoot)
if ($stopReason) { Write-Host "STOPPED: $stopReason" -ForegroundColor Red }
Write-Host @"

Review the integrated result, then adopt it yourself:
  git log --oneline $baseSha..$integBranch
  git diff --stat $baseSha...$integBranch
  git merge --no-ff $integBranch
Rerun failures:  pwsh -NoProfile -File scripts/task-pack/run.ps1 -Task <ID> -Fresh
"@
if ($stopReason) { exit 1 }
