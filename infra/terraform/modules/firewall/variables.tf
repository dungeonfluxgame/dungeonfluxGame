variable "name_prefix" {
  description = "Prefix for firewall names."
  type        = string
}

variable "vpc_cidr" {
  description = "VPC CIDR; private traffic (database) is allowed only to this range."
  type        = string
}

variable "app_tag" {
  description = "Tag carried by every application Droplet."
  type        = string
}

variable "worker_tag" {
  description = "Tag carried by every worker Droplet."
  type        = string
}

variable "bastion_tag" {
  description = "Tag carried by the bastion Droplet, or an empty string when there is no bastion. SSH to application and worker Droplets is accepted ONLY from this tag; with no bastion there is no SSH rule at all (console access only)."
  type        = string
  default     = ""
}

variable "ssh_port" {
  description = "SSH port."
  type        = number
  default     = 22
}

variable "public_http_sources" {
  description = "Who may reach ports 80 and 443 on application Droplets when no load balancer is used."
  type        = list(string)
  default     = ["0.0.0.0/0", "::/0"]
}

variable "load_balancer_ids" {
  description = "When non-empty, ports 80/443 accept traffic only from these load balancers and public_http_sources is ignored (plan 12.4 topology)."
  type        = list(string)
  default     = []
}

variable "runtime_port_range" {
  description = "TCP port range on worker Droplets that the control plane dials to reach campaign runtimes (the game tunnel). Only the application tier may reach it. TODO(O1): confirm the runtime port model (one process per campaign needs one listener per campaign)."
  type        = string
  default     = "20000-20999"
}

variable "media_gateway_port" {
  description = "Port of the control-plane media gateway (internal mTLS; LAUNCH.md decision 23) that a worker-hosted runtime dials. TODO(O1,O2): confirm."
  type        = string
  default     = "8083"
}
