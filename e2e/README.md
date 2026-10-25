# Local flow acceptance

Python 3.11+ and the installed Microsoft Edge are used. No test is skipped or expected-failed
because the product is unfinished. `FAIL`, `MISSING` and `BLOCKED` all make the runner exit 1
and name a gap. A `PASS` is only the specific assertion described in its result row.
`flows.md` and the round report remain the authoritative coverage/residual maps.

From PowerShell, start every shell in the repository:

```powershell
Set-Location 'C:\Users\mreca\Desktop\dungeonfluxGame'
$env:PIP_CACHE_DIR="$PWD/artifacts/cache/pip"
$env:PYTHONPYCACHEPREFIX="$PWD/artifacts/cache/flowmap-pycache"
python -m venv artifacts/cache/flowmap-venv
artifacts/cache/flowmap-venv/Scripts/python -m pip install -r e2e/requirements.txt
```

No browser download is required. The runner defaults to `--channel msedge`. It blocks browser
requests outside the three supplied loopback origins, disables service workers, and uses an
isolated browser context per case. Never point it at the public site or a production account.
Fonts hosted by Google are blocked, so font fidelity needs a separate approved offline-font fixture.

Build the real portal and browser app using the pinned toolchain and required shared target:

```powershell
$env:CARGO_TARGET_DIR='C:/Users/mreca/Desktop/dungeonfluxGame/artifacts/build/agents'
$env:RUSTUP_HOME='C:/Users/mreca/Desktop/dungeonfluxGame/artifacts/cache/rustup'
$env:CARGO_HOME='C:/Users/mreca/Desktop/dungeonfluxGame/artifacts/cache/cargo'
$env:CARGO_BUILD_JOBS='2'
cargo build -p df-hosted --bin df-portal-hosted --offline
$env:CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER='C:/Program Files/LLVM/bin/wasm-ld.exe'
$env:OUT='artifacts/tmp/agents/FLOWMAP/release-bundle'
bash crates/df-account-app/build-web.sh
python -m e2e.prepare prepare
```

`prepare` uses the existing PostgreSQL 18 at 127.0.0.1:55911, creates only a random `flowmap_`
database, renders copied `docs/` with open and closed settings through the actual deployment
renderer and writes source/asset hashes. Its generated bundle is explicitly diagnostic: it can
inspect the compiled WASM when the release size gate fails, and never changes or approves that
gate. Use a gate-passing bundle for release acceptance. Retained output is not overwritten by a
second `prepare`; review/retain the prior run before starting a new attempt.

In an owned shell, with no provider credentials in its environment, start the native account
server (this is not a game server). Do not read a real `.env`:

```powershell
Set-Location 'C:\Users\mreca\Desktop\dungeonfluxGame'
$state=Get-Content artifacts/tmp/agents/FLOWMAP/stack.json | ConvertFrom-Json
Get-ChildItem Env:DF_* | Remove-Item
$env:DF_PROFILE='development'
$env:DF_ACCOUNTS_OPEN='true'
$env:DF_PORTAL_LISTEN='127.0.0.1:49271'
$env:DF_GAMEPLAY_PUBLIC_URL='http://127.0.0.1:49271'
$env:DF_GAMEPLAY_DATABASE_TLS='loopback-plaintext'
$env:DF_PORTAL_DATABASE_URL="host=127.0.0.1 port=55911 user=postgres dbname=$($state.database)"
$env:DF_PORTAL_MIGRATOR_DATABASE_URL=$env:DF_PORTAL_DATABASE_URL
$env:DF_PORTAL_BUNDLE_DIR="$PWD/artifacts/tmp/agents/FLOWMAP/bundle"
$env:DF_PORTAL_OUTBOX="$PWD/artifacts/tmp/agents/FLOWMAP/outbox"
# Synthetic local-only keys; never use these values in a hosted environment.
$env:DF_CAMPAIGN_IP_KEY='11'*32
$env:DF_CAMPAIGN_CODE_KEY='22'*32
$env:DF_TRIAL_EMAIL_HMAC_KEY='33'*32
artifacts/build/agents/debug/df-portal-hosted.exe
```

This selects the real PostgreSQL stores, local passwords, fake mail outbox and development
checkout. It does **not** select the native Stripe or PayPal adapter. Those require a faithful
loopback provider endpoint configurable in the executable; it is currently a gap. Setting
`sk_test_*` still contacts Stripe and is forbidden. OAuth likewise needs an explicitly local
provider port. Never start a game server under this lane's protocol and never use port 63309.

In two further owned shells, run the rendered static servers:

```powershell
Set-Location 'C:\Users\mreca\Desktop\dungeonfluxGame'
python -m http.server 49272 --bind 127.0.0.1 --directory artifacts/tmp/agents/FLOWMAP/site-open
# In a separate shell, same Set-Location:
python -m http.server 49273 --bind 127.0.0.1 --directory artifacts/tmp/agents/FLOWMAP/site-closed
```

Run all cases or a bounded subset:

```powershell
python -m e2e.run --list
python -m e2e.run
python -m e2e.run --flow C02 --dimension validation --dimension security
python -m e2e.run --flow A02 --dimension permissions
```

One module per flow is under `customer/` or `admin/`. `catalog.py` carries the source-backed
contract, `scenarios.py` declares dimensions, and `harness.py` owns bounded reusable execution.
Each JSON row records the exact scope asserted. No fixture substitution can turn a blocked
whole journey into a pass. Missing Python Playwright blocks browser cases individually while
standard-library HTTP checks still run. Authentication setup uses the real API and synthetic
mail only. HTTP checks deliberately forward Secure cookies over this exact loopback fixture;
they are protocol checks, **not** browser cookie delivery/TLS acceptance. Browser setup preserves
the cookie flags; its fixture injection also does not prove delivery by the server.
Each HTTP client binds a separate synthetic 127/8 source address, with a random prefix per run,
so independent test identities do not exhaust one address's signup throttle. The real throttle
is never disabled. Explicit throttle-abuse cases need their own controlled fixture.

`e2e/serve_portal.ps1` is the equivalent native launch command after copying the built binary
to the attempt's `df-portal-hosted-observed.exe`. It clears inherited provider settings and
records native logs. The retained Node browser probes in the attempt folder are separate
fallback evidence; they do not count as execution of the Python Playwright cases.

Screenshots are retained on browser failure under `artifacts/tmp/agents/FLOWMAP/screens/`.
Do not attach raw request/response bodies, cookies, passwords, mailed tokens, HAR or traces to
reports. Synthetic outbox files remain private test fixtures, not report evidence. Unimplemented
full action journeys, clocks, commit barriers, operator bootstrap, scoped audit reads, runtime,
manual screen-reader/audio and translation-quality review remain explicitly non-passing.

Stop each owned server with Ctrl+C, then remove the scratch database without forced disconnect:

```powershell
Set-Location 'C:\Users\mreca\Desktop\dungeonfluxGame'
python -m e2e.prepare cleanup
```

No product code is changed by this suite. Rust lint/WASM checks apply to the product owners;
this lane additionally runs Python compilation, runner self-checks and its required lane gate.
