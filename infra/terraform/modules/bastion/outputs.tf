output "address" {
  description = "Address to put in the operator's ssh config as the ProxyJump host (reserved IP)."
  value       = one(module.droplet.reserved_ips)
}

output "private_ip" {
  description = "Private VPC address."
  value       = one(module.droplet.private_ips)
}
