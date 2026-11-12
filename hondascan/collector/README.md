# Collector for the opt-in car report

`worker.js` is a Cloudflare Worker that accepts `POST /ingest` (JSON, at most 32 KB, schema 1, a 32-hex install id, no
VIN-shaped string) and writes it to an R2 bucket as `reports/<install id>.json` (one object per install; a newer report
replaces the older), at most 5 a day per install (the count is kept in that object's metadata). A report with no errors is kept only for a car
model, OS and adapter type not seen before: each new one leaves an empty marker `cars/<model>-<os>-<adapter>` (no install id,
not covered by the 12-month rule), and a later error-free report for a known one is answered 204 and dropped. It has no read route and
holds no secret, so a copy of the app cannot learn anything from it. It never reads the sender's address, and logging is off
in `wrangler.toml`. Cloudflare itself carries the connection, as it does for any website it hosts.

This is the server side of `hondascan/src/share.rs`; what the app sends is described in the wiki's Privacy page, and
`hondascan share preview` prints the exact text.

Deploy (a free Cloudflare account is enough; nothing here has been deployed yet):

```
npm install -g wrangler
wrangler login
wrangler r2 bucket create openhonda    # skip if the bucket already exists
wrangler deploy
```

Then:

1. Set a lifecycle rule on the bucket that deletes objects after 12 months (the Privacy page says so).
2. Build releases with the Worker's `/ingest` address in `HONDASCAN_SHARE_ENDPOINT` in the build's environment
   (`release.py` forwards it into the build containers by name, so the value is not printed). It is read at compile time
   (`hondascan/src/share.rs`), so the source holds no address and a build from source has none: it asks nobody anything and
   cannot send. The program never prints the address, and `public-export/export.py` refuses to publish one.
3. If the rate-limit binding is not available on your plan, delete the `[[ratelimits]]` block; the Worker skips the check when
   the binding is absent.

The address cannot be hidden from someone who looks inside a release binary: a program that sends to an address contains it. What
protects the collector is this folder's code: it keeps only a report of the exact expected shape, has no read route, holds no key,
and takes at most 5 reports a day from one install and 5 writes a minute in all.

Read the reports with `wrangler r2 object get` or any S3 client using the bucket's own credentials, which only you hold.
To delete one person's report on request, delete the object `reports/<their install id>.json`
(`wrangler r2 object delete openhonda/reports/<their install id>.json`).
