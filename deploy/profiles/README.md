# Deployment profiles

`production.env` is the hosted production profile. It requires WorkOS identity,
real mail, informational logs, public HTTPS origins, verified database TLS, and
all development controls and fixtures off. Production provider keys are injected
from secret files under `/etc/dungeonflux/*.env`; the profile stores only the
secret directory and credential-file names, never credential values.

`development.env` records the loopback defaults for local work: local-password
identity, captured dev mail, optional fake providers, development controls, and
verbose logs.

`demo.env` is a named production-facing demo profile. It inherits the production
security posture with two documented exceptions: the currently available
`df-transport-fixture --gameplay-demo` executable (the interactive demo entry
point), and loopback plaintext PostgreSQL (the provisioned host database accepts
only local connections). The public edge still uses HTTPS and strict origins.

Pick `production` for a production service, `development` only on a developer
machine, and `demo` only for the separately managed DungeonFlux demo host. The
guard accepts a profile file or a directory of `*.env` files; it never evaluates
shell syntax or prints environment values.

Production also requires the owner-managed `/etc/dungeonflux/business.env` file
with the business identity variables. The repository contains placeholders only;
`DF_SELLER_PUBLISHED` stays `false` until the owner confirms good standing.

```sh
deploy/profiles/check-profile.sh production deploy/profiles/production.env
# Build and inspect the demo image using the named demo exception:
deploy/profiles/check-profile.sh demo deploy/profiles/demo.env ghcr.io/dungeonfluxgame/dungeonflux-demo:tag
```

The first package published under the organization will be private by default. An
organization owner must make it public before the public demo host can pull it.

**Production never runs a dev profile; the guard enforces it.** For now, binary
feature proof is fail-closed: production requires the requested embedded build
metadata contract (see `request.md`). The current demo image is checked under the
explicit `demo` exception instead.
