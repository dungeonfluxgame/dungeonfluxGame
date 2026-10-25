output "bucket_names" {
  description = "Map of purpose suffix to full bucket name."
  value       = { for suffix, bucket in local.all_buckets : suffix => bucket.name }
}

output "endpoint" {
  description = "S3-compatible endpoint for the region."
  value       = "https://${var.region}.digitaloceanspaces.com"
}

output "public_static_cdn_endpoint" {
  description = "CDN endpoint for the public-static bucket, or null."
  value       = var.cdn_for_public_static ? digitalocean_cdn.public_static[0].endpoint : null
}
