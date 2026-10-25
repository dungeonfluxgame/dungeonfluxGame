#!/bin/sh
# Create the buckets listed in deploy/storage/buckets.csv on an S3-compatible endpoint, with versioning and
# ONE lifecycle rule per bucket. The same CSV drives the Terraform Spaces module, so staging and production
# agree. STATUS: UNVERIFIED. Runs inside the aws-cli image (S3 API only; no jq, no MinIO-specific tool).
#
# Environment (this script is not the game: it is a staging helper, so it may read the environment):
# DF_OBJECT_ENDPOINT (https), DF_BUCKET_PREFIX, AWS_DEFAULT_REGION, AWS_CA_BUNDLE, credential FILES
# DF_OBJECT_ACCESS_KEY_ID_FILE / DF_OBJECT_SECRET_KEY_FILE. Lifecycle JSON is generated with printf.
#
# One rule per bucket, never three with the same empty prefix: overlapping rules can be rejected by
# prefix-based lifecycle implementations. Expiry columns: 0 disables that part of the rule.
set -eu

csv=${DF_BUCKETS_CSV:-/opt/dungeonflux/buckets.csv}
endpoint=${DF_OBJECT_ENDPOINT:?}
prefix=${DF_BUCKET_PREFIX:?}
case "$endpoint" in
  https://*) ;;
  *) echo "refusing a non-TLS object-store endpoint: $endpoint" >&2; exit 1 ;;
esac
AWS_ACCESS_KEY_ID=$(cat "${DF_OBJECT_ACCESS_KEY_ID_FILE:?}")
AWS_SECRET_ACCESS_KEY=$(cat "${DF_OBJECT_SECRET_KEY_FILE:?}")
export AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY
s3() { aws --endpoint-url "$endpoint" "$@"; }

# The object store may still be starting.
attempt=0
until s3 s3api list-buckets >/dev/null 2>&1; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 30 ]; then echo "object store not reachable at $endpoint" >&2; exit 1; fi
  sleep 2
done

tail -n +2 "$csv" | while IFS=, read -r suffix acl versioning expire noncurrent abort cors purpose; do
  bucket="$prefix-$suffix"
  echo "bucket $bucket ($purpose)"
  s3 s3api head-bucket --bucket "$bucket" 2>/dev/null || s3 s3api create-bucket --bucket "$bucket"

  if [ "$versioning" = "on" ]; then
    s3 s3api put-bucket-versioning --bucket "$bucket" --versioning-configuration Status=Enabled
  fi

  parts=""
  add_part() { if [ -n "$parts" ]; then parts="$parts,"; fi; parts="$parts$1"; }
  if [ "$expire" != "0" ]; then add_part "$(printf '"Expiration":{"Days":%s}' "$expire")"; fi
  if [ "$noncurrent" != "0" ]; then add_part "$(printf '"NoncurrentVersionExpiration":{"NoncurrentDays":%s}' "$noncurrent")"; fi
  if [ "$abort" != "0" ]; then add_part "$(printf '"AbortIncompleteMultipartUpload":{"DaysAfterInitiation":%s}' "$abort")"; fi
  if [ -n "$parts" ]; then
    s3 s3api put-bucket-lifecycle-configuration --bucket "$bucket" \
      --lifecycle-configuration "{\"Rules\":[{\"ID\":\"lifecycle\",\"Status\":\"Enabled\",\"Filter\":{\"Prefix\":\"\"},$parts}]}"
  fi

  # Public access is a property of the production ACL column only; the staging store keeps everything
  # private (a mirror must not be reachable anonymously). CORS is likewise not mirrored.
  : "$acl" "$cors"
done
echo "bucket-init done"
