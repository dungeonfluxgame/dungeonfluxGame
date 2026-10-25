output "app_firewall_id" {
  description = "Firewall attached to application Droplets by tag."
  value       = digitalocean_firewall.app.id
}

output "worker_firewall_id" {
  description = "Firewall attached to worker Droplets by tag."
  value       = digitalocean_firewall.worker.id
}
