terraform {
  required_version = ">= 1.9.0, < 2.0.0"
  required_providers {
    digitalocean = {
      source  = "digitalocean/digitalocean"
      version = ">= 2.40.0, < 3.0.0"
    }
  }
}
