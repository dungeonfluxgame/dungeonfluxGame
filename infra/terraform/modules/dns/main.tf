# OPTIONAL DNS in DigitalOcean (LAUNCH.md decision 32). The default is NOT to use it: GoDaddy is the registrar and
# the owner enters the records in deploy/dns/records.csv by hand. Use this module only if the owner later moves DNS
# to DigitalOcean, which requires the owner to change the domain's nameservers at GoDaddy (a registrar setting:
# agents never change it). Nothing here is applied by agents. Existing MX, NS and verification TXT records of the
# apex are NOT managed here; copy them into the zone first or mail and site verification break.
resource "digitalocean_domain" "this" {
  name = var.domain
}

resource "digitalocean_record" "apex" {
  domain = digitalocean_domain.this.id
  type   = "A"
  name   = "@"
  value  = var.edge_ipv4
  ttl    = var.ttl_seconds
}

resource "digitalocean_record" "www" {
  domain = digitalocean_domain.this.id
  type   = "CNAME"
  name   = var.www_label
  value  = "@"
  ttl    = var.ttl_seconds
}

resource "digitalocean_record" "staging" {
  count  = var.staging_ipv4 == null ? 0 : 1
  domain = digitalocean_domain.this.id
  type   = "A"
  name   = var.staging_label
  value  = var.staging_ipv4
  ttl    = 600
}

resource "digitalocean_record" "mail_spf" {
  count  = var.mail_spf_include == "" ? 0 : 1
  domain = digitalocean_domain.this.id
  type   = "TXT"
  name   = var.mail_label
  value  = "v=spf1 include:${var.mail_spf_include} -all"
  ttl    = 3600
}

resource "digitalocean_record" "mail_dkim" {
  for_each = var.mail_dkim_records
  domain   = digitalocean_domain.this.id
  type     = each.value.type
  name     = "${each.key}._domainkey.${var.mail_label}"
  value    = each.value.value
  ttl      = 3600
}

resource "digitalocean_record" "mail_dmarc" {
  domain = digitalocean_domain.this.id
  type   = "TXT"
  name   = "_dmarc.${var.mail_label}"
  value  = var.dmarc_report_address == "" ? "v=DMARC1; p=${var.dmarc_policy}" : "v=DMARC1; p=${var.dmarc_policy}; rua=mailto:${var.dmarc_report_address}"
  ttl    = 3600
}
