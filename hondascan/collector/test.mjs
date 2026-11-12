// Posts the report in the file named on the command line to worker.js with a fake bucket and prints the status and what was
// stored: `node test.mjs report.json [times]`, posting it `times` times (default 1) to one fake bucket, which keeps each object's
// metadata as R2 does (the per-install daily count lives there). hondascan/tests/share.rs runs it on a real report, so the app and
// the collector cannot drift apart unnoticed.
import { readFileSync } from "node:fs";
import worker from "./worker.js";

const stored = [];
const objects = new Map();
const env = {
  BUCKET: {
    put: async (key, body, opts = {}) => { stored.push({ key, bytes: body.length }); objects.set(key, { customMetadata: opts.customMetadata || {} }); },
    head: async (key) => objects.get(key) || null,
  },
};
const body = readFileSync(process.argv[2], "utf8");
const statuses = [];
let r;
for (let i = 0; i < Number(process.argv[3] || 1); i++) {
  r = await worker.fetch(new Request("https://collector.invalid/ingest", { method: "POST", headers: { "content-type": "application/json" }, body }), env);
  statuses.push(r.status);
}
console.log(JSON.stringify({ status: r.status, text: await r.clone().text(), stored, statuses }));
