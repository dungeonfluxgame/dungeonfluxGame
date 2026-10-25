variable "name_prefix" {
  description = "Prefix for every resource name in this environment, for example dfx-prod."
  type        = string
}

variable "region" {
  description = "DigitalOcean region slug. Application, database and Spaces must be jointly available there (plan 12.2)."
  type        = string
}

variable "ip_range" {
  description = "Private CIDR of the VPC. Must not overlap another VPC you peer with."
  type        = string

  validation {
    condition     = can(cidrhost(var.ip_range, 0))
    error_message = "ip_range must be a valid CIDR block."
  }
}
