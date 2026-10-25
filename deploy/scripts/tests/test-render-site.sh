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
grep -Fq 'DungeonFlux est exploité par EXAMPLE &amp; &lt;GROUP&gt;, limited liability company.' "$site_dir/fr/index.html"
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
# Legal-page entity hooks, run against the real docs tree with a sample operator.
legal_env="$temporary_dir/legal.env"
cat > "$legal_env" <<'ENV'
DF_LEGAL_ENTITY_NAME=Example Studio LLC
DF_LEGAL_ENTITY_KIND=a Florida limited liability company
DF_LEGAL_ENTITY_KIND_ES=una sociedad de responsabilidad limitada constituida en Florida
DF_LEGAL_ENTITY_KIND_DE=eine Limited Liability Company nach dem Recht des US-Bundesstaates Florida
DF_LEGAL_ENTITY_KIND_FR=une société à responsabilité limitée de droit floridien
DF_TRADING_NAME=DungeonFlux
DF_SUPPORT_EMAIL=support@example.test
DF_BILLING_EMAIL=billing@example.test
DF_PRIVACY_EMAIL=privacy@example.test
DF_SECURITY_EMAIL=security@example.test
DF_STATEMENT_DESCRIPTOR=DUNGEONFLUX
DF_SELLER_PUBLISHED=false
ENV

unpublished_site="$temporary_dir/legal-unpublished"
cp -a docs "$unpublished_site"
bash "$root/deploy/scripts/render-site.sh" "$unpublished_site" "$legal_env"
! grep -RFq 'Example Studio' "$unpublished_site"
! grep -RFq 'Florida' "$unpublished_site"
! grep -RFq 'data-df="entity-copy"' "$unpublished_site"
! grep -RFq '{{entity}}' "$unpublished_site"
grep -Fq 'The company responsible for your data remains to be confirmed.' "$unpublished_site/legal/privacy/index.html"
grep -Fq 'por confirmar.' "$unpublished_site/es/legal/privacy/index.html"
grep -Fq 'muss noch benannt werden.' "$unpublished_site/de/legal/privacy/index.html"
grep -Fq 'reste à confirmer.' "$unpublished_site/fr/legal/privacy/index.html"
grep -Fq 'Seller entity, address and contact details.' "$unpublished_site/legal/terms/index.html"

published_env="$temporary_dir/legal-published.env"
sed 's/DF_SELLER_PUBLISHED=false/DF_SELLER_PUBLISHED=true/' "$legal_env" > "$published_env"
published_site="$temporary_dir/legal-published"
cp -a docs "$published_site"
bash "$root/deploy/scripts/render-site.sh" "$published_site" "$published_env"
grep -Fq 'DungeonFlux is operated by Example Studio LLC, a Florida limited liability company.' "$published_site/legal/privacy/index.html"
grep -Fq 'DungeonFlux wird betrieben von Example Studio LLC, eine Limited Liability Company nach dem Recht des US-Bundesstaates Florida.' "$published_site/de/legal/privacy/index.html"
grep -Fq 'DungeonFlux es operado por Example Studio LLC, una sociedad de responsabilidad limitada constituida en Florida.' "$published_site/es/legal/privacy/index.html"
grep -Fq 'DungeonFlux est exploité par Example Studio LLC, une société à responsabilité limitée de droit floridien.' "$published_site/fr/legal/privacy/index.html"
grep -Fq 'The company responsible for your data is Example Studio LLC, a Florida limited liability company.' "$published_site/legal/privacy/index.html"
declare -A legal_kinds=([en]='a Florida limited liability company' [es]='una sociedad de responsabilidad limitada constituida en Florida' [de]='eine Limited Liability Company nach dem Recht des US-Bundesstaates Florida' [fr]='une société à responsabilité limitée de droit floridien')
for language in en de es fr; do
  prefix=''
  [[ "$language" == en ]] || prefix="$language/"
  for page in privacy terms refunds; do
    file="$published_site/${prefix}legal/$page/index.html"
    grep -Fq 'Example Studio LLC' "$file"
    grep -Fq "Example Studio LLC, ${legal_kinds[$language]}" "$file"
  done
done
! grep -Fq 'remains to be confirmed' "$published_site/legal/privacy/index.html"
! grep -RFq 'data-df="entity-copy"' "$published_site"
! grep -RFq '{{entity}}' "$published_site"
! grep -RFq '{{kind}}' "$published_site"

printf '%s\n' 'site renderer tests passed'
