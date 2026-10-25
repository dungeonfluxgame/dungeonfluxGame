#!/usr/bin/env bash
set -Eeuo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)
cd -- "$root"
unset DF_ACCOUNTS_OPEN
temporary_dir=$(mktemp -d artifacts/tmp/agents/ACCOUNTSCLOSED-site.XXXXXX)
trap 'rm -rf "$temporary_dir"' EXIT
cat > "$temporary_dir/business.env" <<'ENV'
DF_LEGAL_ENTITY_NAME=Example LLC
DF_LEGAL_ENTITY_KIND=limited liability company
DF_TRADING_NAME=DungeonFlux
DF_SUPPORT_EMAIL=support@example.test
DF_BILLING_EMAIL=billing@example.test
DF_PRIVACY_EMAIL=privacy@example.test
DF_SECURITY_EMAIL=security@example.test
DF_STATEMENT_DESCRIPTOR=DUNGEONFLUX
DF_SELLER_PUBLISHED=false
DF_ACCOUNTS_OPEN=false
ENV

cp -a docs "$temporary_dir/closed"
bash deploy/scripts/render-site.sh "$temporary_dir/closed" "$temporary_dir/business.env"
closed_failed=false
if grep -REq 'href="/app/sign-(up|in)"' "$temporary_dir/closed" --include='*.html'; then
  printf '%s\n' 'closed site contains an account link' >&2
  closed_failed=true
fi
while IFS= read -r page; do
  grep -Eq '<nav class="bar-nav"' "$page" || continue
  if ! grep -Eq '<small>(Coming soon|Pr&oacute;ximamente|Demn&auml;chst|Bient&ocirc;t)</small>' "$page" && ! grep -Eiq '<h1[^>]*>(Sign-(in|up) is coming soon|El inicio de sesi&oacute;n llegar&aacute; pronto|El registro llegar&aacute; pronto|Anmeldung bald verf&uuml;gbar|Registrierung bald verf&uuml;gbar|Connexion bient&ocirc;t disponible|Inscription bient&ocirc;t disponible)</h1>' "$page"; then
    printf 'closed site has no coming-soon label: %s\n' "$page" >&2
    closed_failed=true
  fi
  if [[ "$page" != *"/app/"* ]] && ! grep -Eq '<span class="account-closed" aria-disabled="true">[^<]+<small>' "$page"; then
    printf 'closed site lost its noninteractive account control: %s
' "$page" >&2
    closed_failed=true
  fi
done < <(find "$temporary_dir/closed" -type f -name '*.html' -print)

cp -a docs "$temporary_dir/open"
DF_ACCOUNTS_OPEN=true bash deploy/scripts/render-site.sh "$temporary_dir/open" "$temporary_dir/business.env"
grep -REq 'href="/app/sign-in"' "$temporary_dir/open" --include='*.html'
grep -REq 'href="/app/sign-up"' "$temporary_dir/open" --include='*.html'
if grep -REq '<small>(Coming soon|Pr&oacute;ximamente|Demn&auml;chst|Bient&ocirc;t)</small>' "$temporary_dir/open" --include='*.html'; then
  printf '%s\n' 'open site contains a coming-soon account CTA' >&2
  exit 1
fi
if [[ "$closed_failed" == true ]]; then
  exit 1
fi
printf '%s\n' 'site account renderer tests passed'
