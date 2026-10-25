# Buckets for plan 13.2: public-static, private-media, uploads-quarantine, exports, backups, recovery-journal.
# Prefixes inside buckets are organisational only; authorization is the database records plus per-bucket
# access keys (deploy/secrets/secrets.manifest.json). Spaces has no object lock: no bucket here is WORM, and
# nothing may be described as immutable storage (plan 13.10). Per LAUNCH.md decision 23 all media bytes go
# through the control-plane media gateway, never presigned object-store URLs, so no browser CORS rule is
# needed for the private buckets (the CSV marks them "none"; only public-static serves browsers directly).
#
# Access keys are NOT created here. Create one key per credential domain (media, exports, backups writer,
# journal) in the control panel with per-bucket grants, or with a provider resource if your provider version
# supports it, and store them per the secrets manifest. TODO(owner): verify the per-bucket grant model.
#
# prevent_destroy cannot be conditional, so each bucket is declared twice and exactly one copy exists:
# "protected" (production) refuses destruction, "unprotected" (staging, coordinator decision c) can be torn down.
locals {
  rows    = csvdecode(file(var.buckets_csv))
  buckets = {
    for row in local.rows : row.suffix => {
      acl                    = row.acl
      versioning             = row.versioning == "on"
      expire_days            = tonumber(row.expire_days)
      noncurrent_expire_days = tonumber(row.noncurrent_expire_days)
      abort_multipart_days   = tonumber(row.abort_multipart_days)
      cors                   = row.cors
    }
  }

  cors_methods = {
    read   = ["GET", "HEAD"]
    upload = ["GET", "HEAD", "PUT", "POST"]
  }

  all_buckets = merge(digitalocean_spaces_bucket.protected, digitalocean_spaces_bucket.unprotected)
}

resource "digitalocean_spaces_bucket" "protected" {
  for_each = var.prevent_destroy ? local.buckets : {}

  name          = "${var.bucket_prefix}-${each.key}"
  region        = var.region
  acl           = each.value.acl
  force_destroy = false

  versioning {
    enabled = each.value.versioning
  }

  # ONE lifecycle rule per bucket: several rules with the same empty prefix can be rejected as overlapping.
  dynamic "lifecycle_rule" {
    for_each = (each.value.expire_days > 0 || each.value.noncurrent_expire_days > 0 || each.value.abort_multipart_days > 0) ? [1] : []
    content {
      id      = "lifecycle"
      enabled = true
      prefix  = ""

      dynamic "expiration" {
        for_each = each.value.expire_days > 0 ? [each.value.expire_days] : []
        content {
          days = expiration.value
        }
      }

      dynamic "noncurrent_version_expiration" {
        for_each = each.value.noncurrent_expire_days > 0 ? [each.value.noncurrent_expire_days] : []
        content {
          days = noncurrent_version_expiration.value
        }
      }

      abort_incomplete_multipart_upload_days = each.value.abort_multipart_days > 0 ? each.value.abort_multipart_days : null
    }
  }

  dynamic "cors_rule" {
    for_each = contains(keys(local.cors_methods), each.value.cors) && length(var.cors_allowed_origins) > 0 ? [each.value.cors] : []
    content {
      allowed_methods = local.cors_methods[cors_rule.value]
      allowed_origins = var.cors_allowed_origins
      allowed_headers = ["*"]
      max_age_seconds = 600
    }
  }

  lifecycle {
    prevent_destroy = true
  }
}

resource "digitalocean_spaces_bucket" "unprotected" {
  for_each = var.prevent_destroy ? {} : local.buckets

  name          = "${var.bucket_prefix}-${each.key}"
  region        = var.region
  acl           = each.value.acl
  force_destroy = false

  versioning {
    enabled = each.value.versioning
  }

  # ONE lifecycle rule per bucket: several rules with the same empty prefix can be rejected as overlapping.
  dynamic "lifecycle_rule" {
    for_each = (each.value.expire_days > 0 || each.value.noncurrent_expire_days > 0 || each.value.abort_multipart_days > 0) ? [1] : []
    content {
      id      = "lifecycle"
      enabled = true
      prefix  = ""

      dynamic "expiration" {
        for_each = each.value.expire_days > 0 ? [each.value.expire_days] : []
        content {
          days = expiration.value
        }
      }

      dynamic "noncurrent_version_expiration" {
        for_each = each.value.noncurrent_expire_days > 0 ? [each.value.noncurrent_expire_days] : []
        content {
          days = noncurrent_version_expiration.value
        }
      }

      abort_incomplete_multipart_upload_days = each.value.abort_multipart_days > 0 ? each.value.abort_multipart_days : null
    }
  }

  dynamic "cors_rule" {
    for_each = contains(keys(local.cors_methods), each.value.cors) && length(var.cors_allowed_origins) > 0 ? [each.value.cors] : []
    content {
      allowed_methods = local.cors_methods[cors_rule.value]
      allowed_origins = var.cors_allowed_origins
      allowed_headers = ["*"]
      max_age_seconds = 600
    }
  }
}

resource "digitalocean_cdn" "public_static" {
  count  = var.cdn_for_public_static ? 1 : 0
  origin = local.all_buckets["public-static"].bucket_domain_name
}
