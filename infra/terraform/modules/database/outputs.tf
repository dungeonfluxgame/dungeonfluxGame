output "private_host" {
  description = "Private endpoint hostname inside the VPC."
  value       = local.cluster_private_host
}

output "port" {
  description = "Direct connection port."
  value       = local.cluster_port
}

output "database_name" {
  description = "Application database."
  value       = digitalocean_database_db.app.name
}

output "ca_certificate" {
  description = "PEM CA certificate for tls verify-full. Public data."
  value       = data.digitalocean_database_ca.this.certificate
}

# There are deliberately NO password outputs. Service roles are created by deploy/db/create-roles.sh with
# operator-generated passwords. The cluster administrator's credential exists only in the DigitalOcean control
# panel and in Terraform state (see the note at the top of main.tf).
