param([string]$Binary = 'artifacts/tmp/agents/FLOWMAP/df-portal-hosted-observed.exe')
$ErrorActionPreference = 'Stop'
Set-Location 'C:\Users\mreca\Desktop\dungeonfluxGame'
$flowmapState = Get-Content artifacts/tmp/agents/FLOWMAP/stack.json | ConvertFrom-Json
if ($flowmapState.database -notmatch '^flowmap_[0-9a-f]+$') { throw 'Invalid lane database' }
Get-ChildItem Env:DF_* | Remove-Item
$env:DF_PROFILE = 'development'
$env:DF_ACCOUNTS_OPEN = 'true'
$env:DF_PORTAL_LISTEN = '127.0.0.1:49271'
$env:DF_GAMEPLAY_PUBLIC_URL = 'http://127.0.0.1:49271'
$env:DF_GAMEPLAY_DATABASE_TLS = 'loopback-plaintext'
$env:DF_PORTAL_DATABASE_URL = "host=127.0.0.1 port=55911 user=postgres dbname=$($flowmapState.database)"
$env:DF_PORTAL_MIGRATOR_DATABASE_URL = $env:DF_PORTAL_DATABASE_URL
$env:DF_PORTAL_BUNDLE_DIR = "$PWD/artifacts/tmp/agents/FLOWMAP/bundle"
$env:DF_PORTAL_OUTBOX = "$PWD/artifacts/tmp/agents/FLOWMAP/outbox"
$env:DF_TELEMETRY_DIR = "$PWD/artifacts/tmp/agents/FLOWMAP/telemetry"
$env:DF_CAMPAIGN_IP_KEY = '11' * 32
$env:DF_CAMPAIGN_CODE_KEY = '22' * 32
$env:DF_TRIAL_EMAIL_HMAC_KEY = '33' * 32
& $Binary *> artifacts/tmp/agents/FLOWMAP/portal.log
