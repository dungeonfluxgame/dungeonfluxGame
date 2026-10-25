#!/usr/bin/env bash
set -Eeuo pipefail

fail() { printf 'site renderer: %s\n' "$1" >&2; exit 1; }
[[ $# -ge 1 && $# -le 2 ]] || fail 'usage: render-site.sh <site-directory> [business-env-file]'
site_dir=$1
env_file=${2:-/etc/dungeonflux/business.env}
[[ -d "$site_dir" ]] || fail 'site directory does not exist'
[[ -r "$env_file" ]] || fail 'business environment file is missing or unreadable'

declare -A values=()
while IFS= read -r line || [[ -n "$line" ]]; do
  [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
  [[ "$line" == *=* ]] || fail 'business environment file has a malformed line'
  key=${line%%=*}
  value=${line#*=}
  key=${key//[[:space:]]/}
  [[ "$key" =~ ^DF_(LEGAL_ENTITY_NAME|LEGAL_ENTITY_KIND|TRADING_NAME|SUPPORT_EMAIL|BILLING_EMAIL|PRIVACY_EMAIL|SECURITY_EMAIL|STATEMENT_DESCRIPTOR|SELLER_PUBLISHED|PILOT_OPEN)$ ]] || continue
  [[ ! -v "values[$key]" ]] || fail "business environment repeats $key"
  values["$key"]=$value
done < "$env_file"

for key in DF_LEGAL_ENTITY_NAME DF_LEGAL_ENTITY_KIND DF_TRADING_NAME DF_SUPPORT_EMAIL DF_BILLING_EMAIL DF_PRIVACY_EMAIL DF_SECURITY_EMAIL DF_STATEMENT_DESCRIPTOR DF_SELLER_PUBLISHED; do
  [[ -n ${values[$key]-} ]] || fail "$key is missing"
done
[[ ${values[DF_SELLER_PUBLISHED]} == true || ${values[DF_SELLER_PUBLISHED]} == false ]] || fail 'DF_SELLER_PUBLISHED must be true or false'
pilot_open=${DF_PILOT_OPEN:-${values[DF_PILOT_OPEN]:-false}}
[[ $pilot_open == true || $pilot_open == false ]] || fail 'DF_PILOT_OPEN must be true or false'
[[ ${#values[DF_STATEMENT_DESCRIPTOR]} -ge 5 && ${#values[DF_STATEMENT_DESCRIPTOR]} -le 22 ]] || fail 'DF_STATEMENT_DESCRIPTOR must be 5-22 characters'
[[ ${values[DF_STATEMENT_DESCRIPTOR]} =~ ^[A-Z0-9\ -]+$ ]] || fail 'DF_STATEMENT_DESCRIPTOR has invalid characters'
email_pattern='^[^[:space:]<>@]+@[^[:space:]<>@]+\.[^[:space:]<>@]+$'
for key in DF_SUPPORT_EMAIL DF_BILLING_EMAIL DF_PRIVACY_EMAIL DF_SECURITY_EMAIL; do
  [[ ${values[$key]} =~ $email_pattern ]] || fail "$key is invalid"
done
for key in DF_LEGAL_ENTITY_NAME DF_LEGAL_ENTITY_KIND DF_TRADING_NAME; do
  [[ ${values[$key]} != *$'\n'* && ${values[$key]} != *$'\r'* && -n ${values[$key]//[[:space:]]/} ]] || fail "$key is invalid"
done

html_escape() {
  local value=$1
  value=${value//&/\&amp;}
  value=${value//</\&lt;}
  value=${value//>/\&gt;}
  value=${value//\"/\&quot;}
  printf '%s' "$value"
}

while IFS= read -r -d '' file; do
  export DF_PILOT_STATE=$pilot_open
  perl -0pi -e 's{<span data-df="pilot-copy" data-df-state="\Q$ENV{DF_PILOT_STATE}\E">(.*?)</span>}{$1}gs; s{<span data-df="pilot-copy" data-df-state="(?:open|closed)">.*?</span>}{}gs' "$file"
  case "$file" in
    "$site_dir"/de/*) sentence='wird betrieben von' ;;
    "$site_dir"/es/*) sentence='es operado por' ;;
    "$site_dir"/fr/*) sentence='est exploite par' ;;
    *) sentence='is operated by' ;;
  esac
  if [[ ${values[DF_SELLER_PUBLISHED]} == true ]]; then
    export DF_SELLER_LINE="$(html_escape "${values[DF_TRADING_NAME]}") $sentence $(html_escape "${values[DF_LEGAL_ENTITY_NAME]}")$(html_escape ', ')$(html_escape "${values[DF_LEGAL_ENTITY_KIND]}")."
    perl -0pi -e 's{<p class="seller-line" data-df="seller-line" hidden></p>}{<p class="seller-line" data-df="seller-line">$ENV{DF_SELLER_LINE}</p>}g' "$file"
  else
    perl -0pi -e 's{<p class="seller-line" data-df="seller-line"(?: hidden)?>(.*?)</p>}{<p class="seller-line" data-df="seller-line" hidden></p>}gs' "$file"
  fi
done < <(find "$site_dir" -type f -name '*.html' -print0)

mkdir -p "$site_dir/.well-known"
printf 'Contact: mailto:%s\n' "${values[DF_SECURITY_EMAIL]}" > "$site_dir/.well-known/security.txt"
