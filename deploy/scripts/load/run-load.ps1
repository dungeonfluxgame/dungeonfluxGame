param(
    [Parameter(Mandatory=$true)][string]$WebRoot,
    [string]$OutDir = "artifacts/tmp/agents/LOADHARNESS/windows-run",
    [int]$Seconds = 600,
    [string]$Psql = "artifacts/build/postgres-g05/install/bin/psql.exe"
)
$ErrorActionPreference = "Stop"
Set-Location 'C:/Users/mreca/Desktop/dungeonfluxGame'
$env:CARGO_TARGET_DIR = "C:/Users/mreca/Desktop/dungeonfluxGame/artifacts/build/agents"
$env:RUSTUP_HOME = "C:/Users/mreca/Desktop/dungeonfluxGame/artifacts/cache/rustup"
$env:CARGO_HOME = "C:/Users/mreca/Desktop/dungeonfluxGame/artifacts/cache/cargo"
$env:CARGO_BUILD_JOBS = "2"
cargo build -p df-tools --example load_client --bin df-transport-fixture
if ($LASTEXITCODE -ne 0) { throw "build failed" }
python -B deploy/scripts/load/run_load.py --web-root $WebRoot --out $OutDir --seconds $Seconds --psql $Psql
if ($LASTEXITCODE -ne 0) { throw "load qualification failed; retained logs identify the failed boundary" }
