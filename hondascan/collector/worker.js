// Collector for hondascan's opt-in car reports (hondascan/src/share.rs).
// A Cloudflare Worker with one R2 bucket bound as BUCKET. It is write-only from the client's side: nothing it stores can be
// read back through it, and it holds no secret the app could leak. It never looks at the sender's address (no cf-connecting-ip,
// no x-forwarded-for, no request.cf), and it stores the report exactly as received, under an install id the app made up.
// Built to stay inside Cloudflare's free allowances and to hold nothing but reports: one object per install (a newer report replaces
// the older one), a 32 KB cap, a shape check, and a limit on writes a minute for everyone together. It holds no key: the bucket is
// reached through the binding, so there is no credential to leak, and the bucket itself is never public.
// Deploy: see README.md in this folder.
const MAX_BYTES = 32 * 1024; // the same number as share::MAX_BYTES (a real report is a few KB)
const INSTALL_ID = /^[0-9a-f]{32}$/;
const VIN_SHAPE = /[A-HJ-NPR-Z0-9]{17}/; // the client replaces these with <vin>; a report that still has one is refused

// What a report may hold, as the app builds it (share.rs): exactly these keys, bounded lists, short strings. Anything else is
// refused, so the bucket can only ever hold something that looks like a report.
const TOP = ["schema", "kind", "version", "os", "arch", "install_id", "cars"];
const CAR = ["car", "ecus", "codes", "sessions"];
const LIMITS = { cars: 5, ecus: 64, codes: 128, sessions: 5 };
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

function valid(doc) {
  if (!sameKeys(doc, TOP) || doc.kind !== "hondascan-car-report" || doc.schema !== 1) return "unknown report";
  if (!INSTALL_ID.test(String(doc.install_id))) return "bad install id";
  if (!Array.isArray(doc.cars) || doc.cars.length === 0 || doc.cars.length > LIMITS.cars) return "bad cars";
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
    // One limit for everyone together (not per address, which is never read): it bounds the bucket writes a flood could cause.
    // The app tries again later on a 429. Bound in wrangler.toml; skipped when the binding is absent.
    if (env.LIMITER && !(await env.LIMITER.limit({ key: "all" })).success) return new Response("slow down", { status: 429 });

    await env.BUCKET.put(`reports/${doc.install_id}.json`, body, { httpMetadata: { contentType: "application/json" } });
    return new Response(null, { status: 204 });
  },
};
