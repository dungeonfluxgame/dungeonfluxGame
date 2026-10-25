output "id" {
  description = "VPC UUID."
  value       = digitalocean_vpc.this.id
}

output "ip_range" {
  description = "VPC CIDR."
  value       = digitalocean_vpc.this.ip_range
}
