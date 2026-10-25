output "nameservers" {
  description = "DigitalOcean's nameservers for the zone. The OWNER sets these at GoDaddy by hand to move DNS here; nothing else changes the registrar."
  value       = ["ns1.digitalocean.com", "ns2.digitalocean.com", "ns3.digitalocean.com"]
}

output "hostnames" {
  description = "The hostnames this module publishes."
  value       = {
    apex    = var.domain
    www     = "${var.www_label}.${var.domain}"
    staging = var.staging_ipv4 == null ? null : "${var.staging_label}.${var.domain}"
    mail    = "${var.mail_label}.${var.domain}"
  }
}
