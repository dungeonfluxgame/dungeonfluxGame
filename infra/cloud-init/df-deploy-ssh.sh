#!/bin/bash
# df-deploy-ssh: the FORCED COMMAND of the CI deploy key (coordinator ruling b). authorized_keys for dfdeploy
# carries   command="/usr/local/sbin/df-deploy-ssh",restrict <ci public key>   so that key can do exactly two
# things, whatever the client asks for: push secrets (a tar stream on stdin) and request a release by digest.
# It cannot get a shell, a PTY, a port forward, or run `psql` (database administration is for individual
# operators, with their own account and hardware key). Humans never use this account.
# The client chooses with its command line (SSH_ORIGINAL_COMMAND):
#   secrets-install            ->  sudo df-secrets-install      (tar on stdin)
#   deploy sha256:<64 hex>     ->  sudo df-deploy sha256:<64 hex>
# Everything else is refused. STATUS: UNVERIFIED (never run through sshd).
set -euo pipefail

cmd=${SSH_ORIGINAL_COMMAND:-}
case "$cmd" in
  "secrets-install")
    exec sudo /usr/local/sbin/df-secrets-install
    ;;
  "deploy sha256:"*)
    digest=${cmd#deploy }
    [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || { echo "bad digest" >&2; exit 2; }
    exec sudo /usr/local/sbin/df-deploy "$digest"
    ;;
  *)
    echo "df-deploy-ssh: only 'secrets-install' and 'deploy sha256:<digest>' are allowed" >&2
    exit 2
    ;;
esac
