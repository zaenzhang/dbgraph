$ErrorActionPreference = "Stop"

if (-not $env:DATABASE_URL) {
  $env:DATABASE_URL = "postgres://postgres:postgres@localhost:55432/teashop"
}

$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("dbgraph-postgres-smoke-" + [System.Guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $work | Out-Null
Copy-Item -Recurse -Path (Join-Path $root "examples/postgres-teashop/sql") -Destination (Join-Path $work "sql")
Copy-Item -Path (Join-Path $root "examples/postgres-teashop/schema.sql") -Destination (Join-Path $work "schema.sql")

function Invoke-DbGraphCli {
  param([string[]]$Arguments)

  $output = & cargo run --manifest-path (Join-Path $root "Cargo.toml") -p dbgraph-cli -- @Arguments
  if ($LASTEXITCODE -ne 0) {
    throw "dbgraph command failed: dbgraph $($Arguments -join ' ')"
  }
  $output | Out-Host
  return ($output -join "`n")
}

try {
  Push-Location $work
  Invoke-DbGraphCli @("init", "-i", "--yes") | Out-Null
  $configPath = Join-Path $work ".dbgraph/dbgraph.config.json"
  $config = Get-Content -Raw -Path $configPath | ConvertFrom-Json
  $config.snapshot.profilingMode = "sample"
  $config.snapshot.maxRowsPerTable = 20
  $config.snapshot.sampleRows = $true
  $config | Add-Member -Force -NotePropertyName "dataAccess" -NotePropertyValue ([pscustomobject]@{
    defaultMode = "schemaOnly"
    tables = @(
      [pscustomobject]@{
        pattern = "public.orders"
        mode = "sample"
        columns = @("status", "created_at")
        where = "created_at >= now() - interval '30 days'"
        limit = 10
        storeRawValues = $true
      },
      [pscustomobject]@{
        pattern = "public.payments"
        mode = "schemaOnly"
      }
    )
  })
  $configJson = $config | ConvertTo-Json -Depth 10
  [System.IO.File]::WriteAllText($configPath, $configJson, [System.Text.UTF8Encoding]::new($false))
  Invoke-DbGraphCli @("doctor") | Out-Null
  Invoke-DbGraphCli @("snapshot", "--profile", "sample") | Out-Null
  Invoke-DbGraphCli @("doctor", "--check-db") | Out-Null
  Invoke-DbGraphCli @("search", "orders", "--kind", "table") | Out-Null
  Invoke-DbGraphCli @("validate-sql", "--sql", "select * from orders") | Out-Null
  Invoke-DbGraphCli @("analyze", "--fail-on", "critical") | Out-Null
  $analysis = Invoke-DbGraphCli @("analyze", "--scope", "all", "--json")
  if ($analysis -notmatch "public\.customers\.email") {
    throw "analysis smoke missing expected customer email risk"
  }
  if ($analysis -notmatch "public\.payments\.provider_token") {
    throw "analysis smoke missing expected provider token risk"
  }
  if ($analysis -notmatch "public\.orders\.status") {
    throw "analysis smoke missing expected orders status performance finding"
  }
  if ($analysis -notmatch "data\.enum_like_without_constraint") {
    throw "analysis smoke missing sample-derived data profiling finding"
  }
  if ($analysis -notmatch "Data Profiling") {
    throw "analysis smoke missing data profiling section"
  }
  if ($analysis -notmatch "suggestedFix") {
    throw "analysis smoke missing suggested fixes"
  }
  $benchmark = Invoke-DbGraphCli @("benchmark-agent", "--scenario", "teashop", "--format", "markdown")
  if ($benchmark -notmatch "public\.customers\.email") {
    throw "benchmark smoke missing expected customer email evidence"
  }
  if ($benchmark -notmatch "public\.payments\.provider_token") {
    throw "benchmark smoke missing expected provider token evidence"
  }
  if ($benchmark -notmatch "public\.orders\.status") {
    throw "benchmark smoke missing expected orders status evidence"
  }
  if ($benchmark -notmatch "Token reduction") {
    throw "benchmark smoke missing token reduction summary"
  }
}
finally {
  Pop-Location
  Remove-Item -Recurse -Force $work
}
