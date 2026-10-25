#!/usr/bin/env bash
set -Eeuo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)
cd -- "$root"
unset DF_PILOT_OPEN
temporary_dir=$(mktemp -d artifacts/tmp/agents/BIZCONFIG-render.XXXXXX)
trap 'rm -rf "$temporary_dir"' EXIT
site_dir="$temporary_dir/site"
mkdir -p "$site_dir" "$site_dir/de" "$site_dir/es" "$site_dir/fr"
for language in en de es fr; do
  directory=$site_dir
  [[ "$language" == en ]] || directory="$site_dir/$language"
  printf '%s\n' '<p class="seller-line" data-df="seller-line" hidden></p>' '<p><span data-df="pilot-copy" data-df-state="closed">Closed copy</span><span data-df="pilot-copy" data-df-state="open" hidden>Open copy</span></p>' > "$directory/index.html"
done
cat > "$temporary_dir/business.env" <<'ENV'
DF_LEGAL_ENTITY_NAME=EXAMPLE & <GROUP>
DF_LEGAL_ENTITY_KIND=limited liability company
DF_TRADING_NAME=DungeonFlux
DF_SUPPORT_EMAIL=support@example.test
DF_BILLING_EMAIL=billing@example.test
DF_PRIVACY_EMAIL=privacy@example.test
DF_SECURITY_EMAIL=security@example.test
DF_STATEMENT_DESCRIPTOR=DUNGEONFLUX
DF_SELLER_PUBLISHED=true
ENV

bash "$root/deploy/scripts/render-site.sh" "$site_dir" "$temporary_dir/business.env"
grep -Fq 'DungeonFlux is operated by EXAMPLE &amp; &lt;GROUP&gt;, limited liability company.' "$site_dir/index.html"
grep -Fq 'DungeonFlux wird betrieben von EXAMPLE &amp; &lt;GROUP&gt;, limited liability company.' "$site_dir/de/index.html"
grep -Fq 'DungeonFlux es operado por EXAMPLE &amp; &lt;GROUP&gt;, limited liability company.' "$site_dir/es/index.html"
grep -Fq 'DungeonFlux est exploite par EXAMPLE &amp; &lt;GROUP&gt;, limited liability company.' "$site_dir/fr/index.html"
grep -Fq 'Contact: mailto:security@example.test' "$site_dir/.well-known/security.txt"
for language in en de es fr; do
  directory=$site_dir
  [[ "$language" == en ]] || directory="$site_dir/$language"
  grep -Fq 'Closed copy' "$directory/index.html"
  ! grep -Fq 'Open copy' "$directory/index.html"
  ! grep -Fq 'data-df="pilot-copy"' "$directory/index.html"
done

for language in en de es fr; do
  directory=$site_dir
  [[ "$language" == en ]] || directory="$site_dir/$language"
  printf '%s\n' '<p class="seller-line" data-df="seller-line" hidden></p>' '<p><span data-df="pilot-copy" data-df-state="closed">Closed copy</span><span data-df="pilot-copy" data-df-state="open" hidden>Open copy</span></p>' > "$directory/index.html"
done
DF_PILOT_OPEN=true bash "$root/deploy/scripts/render-site.sh" "$site_dir" "$temporary_dir/business.env"
for language in en de es fr; do
  directory=$site_dir
  [[ "$language" == en ]] || directory="$site_dir/$language"
  grep -Fq 'Open copy' "$directory/index.html"
  ! grep -Fq 'Closed copy' "$directory/index.html"
  ! grep -Fq 'data-df="pilot-copy"' "$directory/index.html"
done

sed -i 's/DF_SELLER_PUBLISHED=true/DF_SELLER_PUBLISHED=false/' "$temporary_dir/business.env"
bash "$root/deploy/scripts/render-site.sh" "$site_dir" "$temporary_dir/business.env"
grep -Fq '<p class="seller-line" data-df="seller-line" hidden></p>' "$site_dir/index.html"
printf '%s\n' 'site renderer tests passed'
