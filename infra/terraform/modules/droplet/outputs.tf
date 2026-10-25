output "ids" {
  description = "Droplet ids (database trusted sources use these)."
  value       = digitalocean_droplet.this[*].id
}

output "private_ips" {
  description = "Private VPC addresses."
  value       = digitalocean_droplet.this[*].ipv4_address_private
}

output "public_ips" {
  description = "Droplet public addresses (administration should not use them; see the firewall)."
  value       = digitalocean_droplet.this[*].ipv4_address
}

output "reserved_ips" {
  description = "Reserved addresses to point DNS at (manual step)."
  value       = digitalocean_reserved_ip.this[*].ip_address
}
