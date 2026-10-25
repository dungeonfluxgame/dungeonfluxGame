variable "domain" {
  description = "The DNS zone, for example dungeonfluxdnd.com (LAUNCH.md decision 32). GoDaddy stays the registrar; this module only matters if the owner moves DNS to DigitalOcean by changing the nameservers at GoDaddy BY HAND. Agents never change DNS or registrar settings."
  type        = string
}

variable "edge_ipv4" {
  description = "Address of the production edge: the load balancer when there is one, otherwise the application Droplet's reserved address. The apex A record points here."
  type        = string
}

variable "www_label" {
  description = "Label of the www host (relative to the zone). It is a CNAME to the apex; the edge answers it with a permanent redirect."
  type        = string
  default     = "www"
}

variable "staging_label" {
  description = "Label of the staging host (staging.dungeonfluxdnd.com by default)."
  type        = string
  default     = "staging"
}

variable "staging_ipv4" {
  description = "Address of the staging edge, or null to publish no staging record."
  type        = string
  default     = null
}

variable "mail_label" {
  description = "Label of the transactional mail subdomain (mail.dungeonfluxdnd.com by default)."
  type        = string
  default     = "mail"
}

variable "mail_spf_include" {
  description = "The mail provider's SPF include domain. Empty publishes no SPF record (the provider is an owner decision)."
  type        = string
  default     = ""
}

variable "mail_dkim_records" {
  description = "DKIM records the mail provider gives, as a map of selector to {type, value}. type is CNAME (value ends with a dot) or TXT."
  type        = map(object({
    type  = string
    value = string
  }))
  default = {}

  validation {
    condition     = alltrue([for k, r in var.mail_dkim_records : contains(["CNAME", "TXT"], r.type)])
    error_message = "A DKIM record is a CNAME or a TXT."
  }
}

variable "dmarc_policy" {
  description = "DMARC policy for the mail subdomain. Start with none to collect reports, then quarantine, then reject."
  type        = string
  default     = "none"

  validation {
    condition     = contains(["none", "quarantine", "reject"], var.dmarc_policy)
    error_message = "dmarc_policy must be none, quarantine or reject."
  }
}

variable "dmarc_report_address" {
  description = "Mailbox for DMARC aggregate reports (an owner decision). Empty publishes the DMARC record without a report address."
  type        = string
  default     = ""
}

variable "ttl_seconds" {
  description = "TTL of the address records. Keep 300 around a cutover and raise it afterwards."
  type        = number
  default     = 300
}
