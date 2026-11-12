// Posts the report in the file named on the command line to worker.js with a fake bucket and prints the status and what was
// stored: `node test.mjs report.json`. hondascan/tests/share.rs runs it on a real report, so the app and the collector cannot
// drift apart unnoticed.
import { readFileSync } from "node:fs";
import worker from "./worker.js";

const stored = [];
const env = { BUCKET: { put: async (key, body) => stored.push({ key, bytes: body.length }) } };
const body = readFileSync(process.argv[2], "utf8");
const r = await worker.fetch(new Request("https://collector.invalid/ingest", { method: "POST", headers: { "content-type": "application/json" }, body }), env);
console.log(JSON.stringify({ status: r.status, text: await r.clone().text(), stored }));
