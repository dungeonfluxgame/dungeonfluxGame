# No secret is an output: not a database password, not a key. Service-role passwords are created outside
# Terraform (deploy/db/create-roles.sh) and never enter state.
output "environment" {
  description = "Environment name."
  value       = local.environment
}

output "bastion_address" {
  description = "ProxyJump host for the operator's ssh config, or null when there is no bastion."
  value       = one(module.bastion[*].address)
}

output "app_reserved_ips" {
  description = "Addresses to point DNS at (a manual owner step)."
  value       = module.app_droplets.reserved_ips
}

output "app_private_ips" {
  description = "Application Droplet private addresses."
  value       = module.app_droplets.private_ips
}

output "worker_private_ips" {
  description = "Worker Droplet private addresses."
  value       = module.worker_droplets.private_ips
}

output "database_private_host" {
  description = "Managed PostgreSQL private hostname (the db-gate forwards to it)."
  value       = module.database.private_host
}

output "database_port" {
  description = "Managed PostgreSQL port."
  value       = module.database.port
}

output "database_name" {
  description = "Application database."
  value       = module.database.database_name
}

output "database_ca_certificate" {
  description = "CA certificate for tls verify-full (public data)."
  value       = module.database.ca_certificate
}

output "object_store_endpoint" {
  description = "Spaces S3 endpoint."
  value       = module.spaces.endpoint
}

output "bucket_names" {
  description = "Purpose suffix to bucket name."
  value       = module.spaces.bucket_names
}

output "edge_public_hostname" {
  description = "The value for DF_PUBLIC_HOST in the edge environment file (deploy/config/edge.env.staging.example)."
  value       = var.public_hostname
}
