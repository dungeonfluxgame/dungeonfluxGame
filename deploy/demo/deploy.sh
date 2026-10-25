#!/usr/bin/env bash
set -Eeuo pipefail

readonly host='root@157.230.52.8'
readonly ssh_key="${HOME}/.ssh/id_ed25519_dungeonfluxdnd"
readonly script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly repository_dir="$(cd -- "$script_dir/../.." && pwd)"
readonly temporary_dir="$(mktemp -d)"
trap 'rm -rf "$temporary_dir"' EXIT

fail() { printf 'deploy: %s\n' "$1" >&2; exit 1; }
if [[ $# -ne 1 ]]; then
  printf '%s\n' 'usage: deploy/demo/deploy.sh <sha256:digest|latest|--rollback>' >&2
  exit 2
fi
[[ -r "$ssh_key" ]] || fail "SSH key is missing or unreadable: $ssh_key"

mode=deploy
digest=$1
if [[ "$digest" == --rollback ]]; then
  mode=rollback
  digest=previous
elif [[ "$digest" == latest ]]; then
  printf '%s\n' 'Resolving latest successful demo-release run on main.'
  run_id="$(gh run list --workflow demo-release.yml --branch main --status success --limit 1 --json databaseId --jq '.[0].databaseId // empty')"
  [[ -n "$run_id" ]] || fail 'No successful demo-release run exists on main.'
  mkdir -p "$temporary_dir/digest"
  gh run download "$run_id" --name demo-digest --dir "$temporary_dir/digest"
  digest="$(<"$temporary_dir/digest/demo-digest.txt")"
fi
if [[ "$mode" == deploy ]]; then
  [[ "$digest" =~ ^sha256:[a-f0-9]{64}$ ]] || fail 'Expected a sha256 digest or latest.'

  public_dir="$temporary_dir/public"
  mkdir -p "$public_dir"
  git -C "$repository_dir" archive HEAD docs | tar -x -C "$public_dir"
  site_dir="$public_dir/docs"
  mkdir -p "$site_dir/app/sign-in" "$site_dir/app/sign-up"
  app_dir="$temporary_dir/app"
  mkdir -p "$app_dir/sign-in" "$app_dir/sign-up"
  fallback_page="$temporary_dir/account-placeholder.html"
  cat > "$fallback_page" <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Accounts | DungeonFlux</title>
</head>
<body>
  <main>
    <h1>Account access</h1>
    <p>Accounts open with the early-access pilot.</p>
  </main>
</body>
</html>
HTML
  for page in index.html sign-in/index.html sign-up/index.html; do
    source_page="$repository_dir/docs/app/$page"
    if [[ ! -s "$source_page" ]]; then
      source_page="$site_dir/app/$page"
    fi
    if [[ ! -s "$source_page" ]]; then
      source_page="$fallback_page"
    fi
    cp "$source_page" "$app_dir/$page"
    cp "$source_page" "$site_dir/app/$page"
  done
  tar -C "$site_dir" -czf "$temporary_dir/site.tar.gz" .
  tar -C "$app_dir" -czf "$temporary_dir/app.tar.gz" .
fi

run_token="$(date -u +%Y%m%d%H%M%S)-$$"
if [[ "$mode" == deploy ]]; then
  scp -i "$ssh_key" "$script_dir/compose.yml" "$host:/tmp/df-demo-$run_token-compose.yml"
  scp -i "$ssh_key" "$script_dir/../scripts/render-site.sh" "$host:/tmp/df-demo-$run_token-render-site.sh"
  scp -i "$ssh_key" "$temporary_dir/site.tar.gz" "$host:/tmp/df-demo-$run_token-site.tar.gz"
  scp -i "$ssh_key" "$temporary_dir/app.tar.gz" "$host:/tmp/df-demo-$run_token-app.tar.gz"
fi

printf 'Deploying to %s.\n' "$host"
ssh -i "$ssh_key" "$host" bash -s -- "$mode" "$digest" "$run_token" <<'REMOTE'
set -Eeuo pipefail
mode=$1
requested_digest=$2
run_token=$3
image='ghcr.io/monstercameron/dungeonflux-demo'
directory='/opt/dungeonflux/demo'
current_file="$directory/current"
previous_file="$directory/previous"
compose_file="$directory/compose.yml"
staging="/tmp/df-demo-deploy-$run_token"
incoming_compose="/tmp/df-demo-$run_token-compose.yml"
incoming_site="/tmp/df-demo-$run_token-site.tar.gz"
incoming_app="/tmp/df-demo-$run_token-app.tar.gz"
incoming_renderer="/tmp/df-demo-$run_token-render-site.sh"

[[ -f /etc/dungeonflux/demo.env ]] || { echo 'Missing owner-managed /etc/dungeonflux/demo.env.' >&2; exit 1; }
[[ -r /etc/dungeonflux/business.env ]] || { echo 'Missing owner-managed /etc/dungeonflux/business.env.' >&2; exit 1; }
install -d -m 0755 "$directory" /opt/dungeonflux
mkdir -m 0700 "$staging"
if [[ "$mode" == rollback ]]; then
  [[ -s "$previous_file" ]] || { echo 'No previous image digest is recorded.' >&2; exit 1; }
  [[ -s "$directory/compose.previous.yml" ]] || { echo 'Previous deployment configuration is incomplete.' >&2; exit 1; }
  target_digest=$(cat "$previous_file")
  cp "$directory/compose.previous.yml" "$staging/compose.next.yml"
else
  target_digest=$requested_digest
  [[ "$target_digest" =~ ^sha256:[a-f0-9]{64}$ ]] || { echo 'Invalid image digest.' >&2; exit 1; }
  [[ -s "$incoming_compose" && -s "$incoming_renderer" && -s "$incoming_site" && -s "$incoming_app" ]] || {
    echo 'Uploaded public site or compose files are missing.' >&2; exit 1;
  }
  cp "$incoming_compose" "$staging/compose.next.yml"
  mkdir "$staging/site.next" "$staging/app.next"
  tar -xzf "$incoming_site" -C "$staging/site.next"
  tar -xzf "$incoming_app" -C "$staging/app.next"
  [[ -s "$staging/site.next/index.html" && -s "$staging/app.next/index.html" ]]
  bash "$incoming_renderer" "$staging/site.next" /etc/dungeonflux/business.env
  if grep -RIEq 'dev@example\.test|test-fakes|Development:' "$staging/site.next" "$staging/app.next"; then
    echo 'Refusing to deploy public files containing development-only content.' >&2
    exit 1
  fi
  chmod -R a+rX "$staging/site.next" "$staging/app.next"
fi
[[ "$target_digest" =~ ^sha256:[a-f0-9]{64}$ ]] || { echo 'Recorded target digest is malformed.' >&2; exit 1; }
old_current=
old_previous=
[[ ! -s "$current_file" ]] || old_current=$(cat "$current_file")
[[ ! -s "$previous_file" ]] || old_previous=$(cat "$previous_file")
for value in "$old_current" "$old_previous"; do
  [[ -z "$value" || "$value" =~ ^sha256:[a-f0-9]{64}$ ]] || { echo 'Recorded digest is malformed.' >&2; exit 1; }
done
[[ ! -f "$compose_file" ]] || cp "$compose_file" "$staging/compose.old.yml"
[[ ! -d /opt/dungeonflux/site ]] || cp -a /opt/dungeonflux/site "$staging/site.old"
[[ ! -d /opt/dungeonflux/app ]] || cp -a /opt/dungeonflux/app "$staging/app.old"
rollback_on_error() {
  result=$?
  trap - ERR
  set +e
  rm -rf /opt/dungeonflux/site /opt/dungeonflux/app
  [[ ! -d "$staging/site.old" ]] || mv "$staging/site.old" /opt/dungeonflux/site
  [[ ! -d "$staging/app.old" ]] || mv "$staging/app.old" /opt/dungeonflux/app
  if [[ -f "$staging/compose.old.yml" ]]; then cp "$staging/compose.old.yml" "$compose_file"; else rm -f "$compose_file"; fi
  if [[ -n "$old_current" ]]; then
    printf '%s\n' "$old_current" > "$current_file"
    DEMO_DIGEST="$old_current" docker compose --project-directory "$directory" -f "$compose_file" up -d
  else
    rm -f "$current_file"
    DEMO_DIGEST="$target_digest" docker compose --project-directory "$directory" -f "$staging/compose.next.yml" down
  fi
  if [[ -n "$old_previous" ]]; then printf '%s\n' "$old_previous" > "$previous_file"; else rm -f "$previous_file"; fi
  rm -f "$incoming_compose" "$incoming_renderer" "$incoming_site" "$incoming_app"
  rm -rf "$staging"
  exit "$result"
}
trap rollback_on_error ERR

if [[ "$mode" == rollback ]]; then
  printf 'Rolling back to %s@%s\n' "$image" "$target_digest"
  docker pull "$image@$target_digest"
else
  printf 'Pulling %s@%s\n' "$image" "$target_digest"
  docker pull "$image@$target_digest"
fi
[[ -z "$old_current" ]] || printf '%s\n' "$old_current" > "$previous_file"
printf '%s\n' "$target_digest" > "$current_file"
install -m 0644 "$staging/compose.next.yml" "$compose_file"
DEMO_DIGEST="$target_digest" docker compose --project-directory "$directory" -f "$compose_file" up -d
healthy=false
for attempt in $(seq 1 36); do
  status=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}missing{{end}}' dungeonflux-demo-game 2>/dev/null || true)
  if [[ "$status" == healthy ]]; then healthy=true; break; fi
  sleep 5
done
[[ "$healthy" == true ]] || { echo 'Game container did not become healthy within three minutes.' >&2; exit 1; }

if [[ "$mode" == deploy ]]; then
  rm -rf /opt/dungeonflux/site /opt/dungeonflux/app
  mv "$staging/site.next" /opt/dungeonflux/site
  mv "$staging/app.next" /opt/dungeonflux/app
fi
nginx -t
systemctl reload nginx
if [[ -f "$staging/compose.old.yml" ]]; then install -m 0644 "$staging/compose.old.yml" "$directory/compose.previous.yml"; else rm -f "$directory/compose.previous.yml"; fi
rm -f "$incoming_compose" "$incoming_renderer" "$incoming_site" "$incoming_app"
rm -rf "$staging"
trap - ERR
printf 'Deployment complete: %s@%s\n' "$image" "$target_digest"
REMOTE
printf '%s\n' 'Deployment command finished.'
