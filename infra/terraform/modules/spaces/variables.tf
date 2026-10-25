variable "region" {
  description = "Spaces region slug. Keep it equal to the application region (plan 12.2)."
  type        = string
}

variable "bucket_prefix" {
  description = "Prefix that makes bucket names unique and environment-specific (coordinator decision g: a variable, with no default in the environment roots). Bucket names are global to the region and 3 to 63 characters."
  type        = string
}

variable "buckets_csv" {
  description = "Path to deploy/storage/buckets.csv, the single definition shared with the staging mirror: suffix, acl, versioning, expiry, noncurrent expiry, multipart abort, cors, purpose."
  type        = string
}

variable "cors_allowed_origins" {
  description = "Browser origins allowed to call buckets whose cors column is read or upload (presigned direct uploads, plan 13.4). Empty means no CORS rule is created, which blocks browser access: fail closed."
  type        = list(string)
  default     = []
}

variable "prevent_destroy" {
  description = "true (production): buckets cannot be destroyed by Terraform. false (staging, coordinator decision c)."
  type        = bool
}

variable "cdn_for_public_static" {
  description = "Create a CDN endpoint for the public-static bucket only. Private media is never put behind the CDN: presigned files get no CDN caching and can double bandwidth cost (plan 13.5)."
  type        = bool
  default     = false
}
