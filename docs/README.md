# Project site

The public project site is live at [dungeonfluxdnd.com](https://dungeonfluxdnd.com).
It has English, German, Spanish and French pages. The static HTML is rendered at
deployment time by `deploy/scripts/render-site.sh` and published to the project droplet
with `deploy/demo/deploy.sh`. It is separate from the Rust game client.

The site's original static HTML/CSS concept material came from the archived
`dungeonflux.old/website/`; its 21 WebP assets are preserved. The original source and
hashes are retained in the task's `work/site-migration/` evidence. The archived local
dice demo has been replaced with a fixed illustrated example and native HTML disclosure;
the site contains no authored JavaScript.

## Local preview

From the project root:

```sh
python3 -m http.server 8766 --bind 127.0.0.1 --directory docs
```

Open `http://127.0.0.1:8766/`. This is a development preview, not an application
runtime. Check section links, native FAQ/example disclosures, gallery scrolling,
asset loading and narrow layouts. The deploy flow renders the pages immediately before
publishing the site.

## Deployment

`deploy/demo/deploy.sh` packages the site and account pages and sends them to the droplet.
The deploy process runs `deploy/scripts/render-site.sh` on the uploaded site using the
host's business settings before publishing it. Keep generated output deploy-time only;
edit the source pages in this directory. The production host, DNS and TLS are managed
separately from this repository.
