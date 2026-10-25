variable "name_prefix" {
  description = "Prefix for the cluster name."
  type        = string
}

variable "region" {
  description = "Region slug; keep it equal to the Droplets' region (plan 12.2)."
  type        = string
}

variable "vpc_id" {
  description = "VPC UUID; the cluster is reachable on its private endpoint."
  type        = string
}

variable "engine_version" {
  description = "PostgreSQL major version. Configuration, not a decision (coordinator decision f): the local development database is 18.x (development/postgres.sh). The owner verifies that DigitalOcean offers this version, and pgvector on it, for the account before the first apply (plan 12.1, 12.4)."
  type        = string

  validation {
    condition     = can(tonumber(var.engine_version)) && tonumber(var.engine_version) >= 17
    error_message = "engine_version must be a PostgreSQL major version of 17 or later (scram verifiers, TLS-only clients and the pinned psql client all assume it)."
  }
}

variable "size" {
  description = "Cluster node size slug. Plan 12.3 pilot: a Standard 2 GiB plan (slug db-s-1vcpu-2gb); plan 12.4 conservative production: an Advanced 8 GiB/2 vCPU primary. The account may not be eligible for the cheaper Standard plans (12.4, owner decision): confirm in the control panel before applying."
  type        = string
}

variable "node_count" {
  description = "1 for the pilot (no standby, explicitly limited service expectations); 2 for primary plus standby, if the account's plan allows (12.4)."
  type        = number

  validation {
    condition     = contains([1, 2, 3], var.node_count)
    error_message = "node_count must be 1, 2 or 3."
  }
}

variable "storage_size_mib" {
  description = "Extra storage in MiB beyond the plan's default, or null to keep the default. Storage is billed separately (12.3)."
  type        = number
  default     = null
}

variable "database_name" {
  description = "Application database name."
  type        = string
}

variable "trusted_droplet_ids" {
  description = "Droplet ids allowed to connect (managed-database trusted sources, plan 12.2)."
  type        = list(string)
}

variable "trusted_ip_cidrs" {
  description = "Additional trusted source addresses, for example the bastion's address during maintenance. Empty by default: the database is not publicly reachable."
  type        = list(string)
  default     = []
}

variable "prevent_destroy" {
  description = "true (production): the cluster cannot be destroyed by Terraform. false (staging, coordinator decision c): it can be torn down."
  type        = bool
}

variable "maintenance_day" {
  description = "Maintenance window day (lowercase weekday)."
  type        = string
  default     = "tuesday"
}

variable "maintenance_hour" {
  description = "Maintenance window start, HH:MM:SS UTC. Choose an hour with no scheduled tables."
  type        = string
  default     = "09:00:00"
}

variable "tags" {
  description = "Tag names for the cluster."
  type        = list(string)
  default     = []
}
