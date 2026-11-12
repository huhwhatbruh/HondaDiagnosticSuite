// Collector for hondascan's opt-in car reports (hondascan/src/share.rs).
// A Cloudflare Worker with one R2 bucket bound as BUCKET. It is write-only from the client's side: nothing it stores can be
// read back through it, and it holds no secret the app could leak. It never looks at the sender's address (no cf-connecting-ip,
// no x-forwarded-for, no request.cf), and it stores the report exactly as received, under an install id the app made up.
// Built to stay inside Cloudflare's free allowances and to hold nothing but reports: one object per install (a newer report replaces
// the older one), a 32 KB cap, a shape check, at most PER_DAY reports per install a day, and a limit on writes a minute for
// everyone together. It holds no key: the bucket is
// reached through the binding, so there is no credential to leak, and the bucket itself is never public.
// Deploy: see README.md in this folder.
const MAX_BYTES = 32 * 1024; // the same number as share::MAX_BYTES (a real report is a few KB)
const INSTALL_ID = /^[0-9a-f]{32}$/;
const PER_DAY = 5; // reports one install may store per UTC day (the app sends at most share::MAX_PER_DAY in 24 hours)
const VIN_SHAPE = /[A-HJ-NPR-Z0-9]{17}/; // the client replaces these with <vin>; a report that still has one is refused

// What a report may hold, as the app builds it (share.rs): exactly these keys, bounded lists, short strings. Anything else is
// refused, so the bucket can only ever hold something that looks like a report.
const TOP = ["schema", "kind", "version", "os", "arch", "install_id", "errors", "cars"];
const ERR = ["method", "adapter", "kind", "message", "count"];
const CAR = ["car", "ecus", "codes", "sessions"];
const LIMITS = { cars: 5, ecus: 64, codes: 128, sessions: 5, errors: 20 };
const MAX_STRING = 200, MAX_KEYS = 200, MAX_DEPTH = 6;

function sameKeys(o, keys) {
  return o !== null && typeof o === "object" && !Array.isArray(o) && Object.keys(o).every((k) => keys.includes(k));
}

// Every string short, every object small, no depth bomb, numbers finite.
function bounded(v, depth = 0) {
  if (depth > MAX_DEPTH) return false;
  if (typeof v === "string") return v.length <= MAX_STRING;
  if (typeof v === "number") return Number.isFinite(v);
  if (v === null || typeof v === "boolean") return true;
  if (Array.isArray(v)) return v.length <= MAX_KEYS && v.every((x) => bounded(x, depth + 1));
  const keys = Object.keys(v);
  return keys.length <= MAX_KEYS && keys.every((k) => k.length <= MAX_STRING) && keys.every((k) => bounded(v[k], depth + 1));
}

// One key per car model, operating system and adapter type with a finished scan (share::car_keys builds the same):
// `1HGCR2F3F-linux-elm`. Only keys made of these characters are used as object names.
const SAFE_KEY = /^[A-Za-z0-9_-]{1,80}$/;
function carKeys(doc) {
  const keys = new Set();
  for (const c of doc.cars) {
    const model = c.car && c.car.vin_read === true ? `${c.car.wmi}${c.car.vds}${c.car.model_year}` : "unknown";
    for (const s of c.sessions) keys.add(`${model}-${doc.os}-${s && s.adapter}`);
  }
  return [...keys].filter((k) => SAFE_KEY.test(k));
}

function valid(doc) {
  if (!sameKeys(doc, TOP) || doc.kind !== "hondascan-car-report" || doc.schema !== 1) return "unknown report";
  if (!INSTALL_ID.test(String(doc.install_id))) return "bad install id";
  // errors (errlog.rs) are optional for reports from before they existed; a report says something: a car, an error or both
  const errors = doc.errors === undefined ? [] : doc.errors;
  if (!Array.isArray(errors) || errors.length > LIMITS.errors || !errors.every((e) => sameKeys(e, ERR))) return "bad errors";
  if (!Array.isArray(doc.cars) || doc.cars.length > LIMITS.cars || doc.cars.length + errors.length === 0) return "bad cars";
  for (const c of doc.cars) {
    if (!sameKeys(c, CAR)) return "bad car";
    for (const k of ["ecus", "codes", "sessions"]) if (!Array.isArray(c[k]) || c[k].length > LIMITS[k]) return "bad " + k;
  }
  return bounded(doc) ? "" : "too much in one field";
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname !== "/ingest") return new Response("not found", { status: 404 });
    if (request.method !== "POST") return new Response("POST only", { status: 405, headers: { Allow: "POST" } });
    if (!(request.headers.get("content-type") || "").startsWith("application/json")) return new Response("JSON only", { status: 415 });
    const declared = Number(request.headers.get("content-length") || "0");
    if (declared > MAX_BYTES) return new Response("too large", { status: 413 });

    const body = await request.text();
    if (body.length > MAX_BYTES) return new Response("too large", { status: 413 });
    let doc;
    try { doc = JSON.parse(body); } catch { return new Response("not JSON", { status: 400 }); }
    const why = valid(doc);
    if (why) return new Response(why, { status: 422 });
    if (VIN_SHAPE.test(body)) return new Response("report still carries a VIN-shaped string", { status: 422 });
    // A report without errors is worth keeping only for a car and platform not seen before (owner, 2026-11-12: one success per
    // car or platform): each new one leaves a marker `cars/<key>` (no install id in it); a report whose cars are all known is
    // accepted and dropped, so the app does not try again.
    const errorFree = (doc.errors || []).length === 0;
    const fresh = [];
    if (errorFree) {
      for (const k of carKeys(doc)) if (!(await env.BUCKET.head(`cars/${k}`))) fresh.push(k);
      if (fresh.length === 0) return new Response(null, { status: 204 });
    }
    // At most PER_DAY reports per install a day, counted on the install's own object (its metadata holds the day and the count),
    // so no other store and no sender address is involved. Checked before the shared limit so a refused install uses none of it.
    const key = `reports/${doc.install_id}.json`;
    const day = new Date().toISOString().slice(0, 10);
    const meta = (await env.BUCKET.head(key))?.customMetadata || {};
    const count = meta.day === day ? Number(meta.count) || 0 : 0;
    if (count >= PER_DAY) return new Response("this install has sent its reports for today", { status: 429 });
    // One limit for everyone together (not per address, which is never read): it bounds the bucket writes a flood could cause.
    // The app tries again later on a 429. Bound in wrangler.toml; skipped when the binding is absent.
    if (env.LIMITER && !(await env.LIMITER.limit({ key: "all" })).success) return new Response("slow down", { status: 429 });

    await env.BUCKET.put(key, body, { httpMetadata: { contentType: "application/json" }, customMetadata: { day, count: String(count + 1) } });
    for (const k of fresh) await env.BUCKET.put(`cars/${k}`, "", { customMetadata: { first: day } });
    return new Response(null, { status: 204 });
  },
};
