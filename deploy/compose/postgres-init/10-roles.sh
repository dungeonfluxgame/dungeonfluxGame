#!/bin/bash
# First-start initialisation of the staging PostgreSQL (docker-entrypoint-initdb.d; runs once, as the
# superuser, over the local socket). Applies the SQL that deploy/scripts/staging-secrets.sh pre-computed on the HOST
# with deploy/db/create-roles.sh: role statements whose passwords are SCRAM-SHA-256 verifiers (never plaintext),
# followed by grants.sql. Computing it on the host keeps the openssl derivation off the database image (which may
# not carry an openssl 3 CLI) and is exactly what an operator ships to production through deploy/db/run-admin-sql.sh.
# Wrapped in a subshell because the entrypoint SOURCES this file when it is not executable. STATUS: UNVERIFIED.
(
  set -euo pipefail
  psql -v ON_ERROR_STOP=1 --no-psqlrc --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" -f /run/secrets/roles.sql
)
