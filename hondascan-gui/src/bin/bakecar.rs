//! `cargo run --release --bin bakecar`: turn the owner's hologram model (`assets/car/car_hologram.glb`: 57 named parts,
//! 217,506 triangles, metres, Y up, nose along +Z, the driver's left at +X) into `assets/car/car_wire.pack`, the only part
//! of it the app embeds. The welcome screen draws the car as a wiregram, so what is kept is the model's feature edges:
//! borders and creases where neighbouring faces turn by more than a threshold (smooth curved panels get no line: that is
//! what keeps it clean), by kind of part. Doing this once here means the app has nothing to parse or search at start-up,
//! uploads a few hundred KB instead of a 7.8 MB model, and ships without the model.
//!
//! Pack, little endian: "CARW", u32 groups; per group u32 vertices, f32 strength, 3 x f32 colour; then the vertices of
//! all groups, 4 x i16 each (x, y, z in 0.1 mm, 0): consecutive pairs are line segments. `ui/carmodel.rs` reads it.
use serde_json::Value;
use std::collections::{hash_map::Entry, HashMap};


/// Drawn in this order: (colour, brightness of its lines, how sharp an edge has to be: the least cosine between the two
/// faces' normals for a line to be drawn; tyres have tread everywhere, so only their sharpest).
const KINDS: [([f32; 3], f32, f32); 4] = [
    ([0.95, 0.50, 0.06], 0.45, 0.55),   // interior: warm orange
    ([0.40, 0.60, 0.80], 0.18, -0.1),   // tyres and wheels: grey-blue
    ([0.06, 0.60, 0.90], 0.75, 0.80),   // body: teal
    ([0.60, 0.85, 1.00], 0.40, 0.55),   // lamps: pale
];

/// Edges of the triangles `idx` (indices into `pos`) that are drawn: borders (one
/// face) and creases (two faces turning past `cos_min`). Vertices are welded by position first, because a mesh with split
/// normals repeats the same point.
fn feature_edges(pos: &[[f32; 3]], idx: &[f64], cos_min: f32, out: &mut Vec<[f32; 3]>) {
    struct E { a: usize, b: usize, n: [f32; 3], count: u32, sharp: bool }
    let mut ids: HashMap<[i32; 3], u32> = HashMap::new();
    let canon: Vec<u32> = pos.iter().map(|p| { let n = ids.len() as u32; *ids.entry([0, 1, 2].map(|k| (p[k] * 2000.0).round() as i32)).or_insert(n) }).collect();
    let mut edges: HashMap<(u32, u32), E> = HashMap::new();
    for t in idx.as_chunks::<3>().0 {
        let v = [t[0] as usize, t[1] as usize, t[2] as usize];
        let d = |a: usize, b: usize| [pos[v[b]][0] - pos[v[a]][0], pos[v[b]][1] - pos[v[a]][1], pos[v[b]][2] - pos[v[a]][2]];
        let (e1, e2) = (d(0, 1), d(0, 2));
        let c = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        if len < 1e-9 { continue; }
        let n = [c[0] / len, c[1] / len, c[2] / len];
        for (x, y) in [(0, 1), (1, 2), (2, 0)] {
            let (ca, cb) = (canon[v[x]], canon[v[y]]);
            if ca == cb { continue; }
            match edges.entry(if ca < cb { (ca, cb) } else { (cb, ca) }) {
                Entry::Occupied(o) => { let e = o.into_mut(); e.count += 1; if (e.n[0] * n[0] + e.n[1] * n[1] + e.n[2] * n[2]).abs() < cos_min { e.sharp = true; } }
                Entry::Vacant(o) => { o.insert(E { a: v[x], b: v[y], n, count: 1, sharp: false }); }
            }
        }
    }
    for e in edges.values().filter(|e| e.count != 2 || e.sharp) { out.extend([pos[e.a], pos[e.b]]); }
}

/// Fewer, longer lines that look the same: duplicate segments (parts share borders) are dropped, runs of segments through a
/// point where exactly two meet are followed as one polyline, simplified (Douglas-Peucker) to within `tol` metres of the
/// original, and polylines shorter than `min_len` are dropped. At 5.6 m across about 900 px a pixel is 6 mm, so 1.5 mm is invisible.
fn simplify(segs: &[[f32; 3]], tol: f32, min_len: f32) -> Vec<[f32; 3]> {
    let key = |p: &[f32; 3]| [0, 1, 2].map(|k| (p[k] * 10_000.0).round() as i32);
    let (mut ids, mut pts): (HashMap<[i32; 3], usize>, Vec<[f32; 3]>) = (HashMap::new(), Vec::new());
    let mut adj: Vec<Vec<(usize, usize)>> = Vec::new();   // (other end, edge number)
    let mut seen: std::collections::HashSet<(usize, usize)> = Default::default();
    let mut ends: Vec<(usize, usize)> = Vec::new();
    for s in segs.as_chunks::<2>().0 {
        let mut id = |p: &[f32; 3]| *ids.entry(key(p)).or_insert_with(|| { pts.push(*p); adj.push(Vec::new()); pts.len() - 1 });
        let (a, b) = (id(&s[0]), id(&s[1]));
        if a == b || !seen.insert((a.min(b), a.max(b))) { continue; }
        adj[a].push((b, ends.len())); adj[b].push((a, ends.len())); ends.push((a, b));
    }
    let mut used = vec![false; ends.len()];
    let mut out = Vec::new();
    let order: Vec<usize> = (0..pts.len()).filter(|&n| adj[n].len() != 2).chain(0..pts.len()).collect();   // open ends and junctions first, then loops
    for start in order {
        for &(first, e0) in &adj[start].clone() {
            if used[e0] { continue; }
            used[e0] = true;
            let mut chain = vec![start, first];
            while adj[*chain.last().unwrap()].len() == 2 {
                let at = *chain.last().unwrap();
                let Some(&(next, e)) = adj[at].iter().find(|&&(_, e)| !used[e]) else { break };
                used[e] = true; chain.push(next);
            }
            let line: Vec<[f32; 3]> = chain.iter().map(|&n| pts[n]).collect();
            let len: f32 = line.windows(2).map(|w| dist(&w[0], &w[1])).sum();
            if len >= min_len { douglas_peucker(&line, tol, &mut out); }
        }
    }
    out
}

fn dist(a: &[f32; 3], b: &[f32; 3]) -> f32 { ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt() }

/// Push `line`'s simplification as vertex pairs.
fn douglas_peucker(line: &[[f32; 3]], tol: f32, out: &mut Vec<[f32; 3]>) {
    let (a, b) = (line[0], line[line.len() - 1]);
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let l2 = ab.iter().map(|v| v * v).sum::<f32>().max(1e-12);
    let off = |p: &[f32; 3]| {   // distance from the segment a-b
        let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1] + (p[2] - a[2]) * ab[2]) / l2).clamp(0.0, 1.0);
        dist(p, &[a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t])
    };
    let (mut worst, mut at) = (0.0f32, 0);
    for (i, p) in line.iter().enumerate().take(line.len() - 1).skip(1) { let d = off(p); if d > worst { worst = d; at = i; } }
    if worst > tol { douglas_peucker(&line[..=at], tol, out); douglas_peucker(&line[at..], tol, out); } else { out.extend([a, b]); }
}

fn kind(name: &str) -> usize {
    let has = |w: &[&str]| w.iter().any(|w| name.contains(w));
    if has(&["Tire", "Wheel_"]) { 1 }
    else if has(&["lamp", "Lamp", "Taillight"]) { 3 }
    else if has(&["Interior", "Seat", "Dash", "Console", "Steering", "Speedometer", "Radio", "Heat_AC", "Information", "Switch", "buck_part"]) { 0 }
    else { 2 }
}

type M = [[f32; 4]; 4];   // row-major, column vectors

fn node_matrix(n: &Value) -> M {
    let f = |k: &str, d: &[f32]| -> Vec<f32> { n[k].as_array().map_or(d.to_vec(), |a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()) };
    if n.get("matrix").is_some() {
        let m = f("matrix", &[]);   // column-major in the file
        return std::array::from_fn(|r| std::array::from_fn(|c| m[c * 4 + r]));
    }
    let (t, q, s) = (f("translation", &[0.0; 3]), f("rotation", &[0.0, 0.0, 0.0, 1.0]), f("scale", &[1.0; 3]));
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let r = [[1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
             [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
             [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)]];
    std::array::from_fn(|i| std::array::from_fn(|j| if j < 3 && i < 3 { r[i][j] * s[j] } else if j == 3 && i < 3 { t[i] } else if i == 3 && j == 3 { 1.0 } else { 0.0 }))
}

/// The bytes of accessor `i` as rows of `width` values (component type 5126 float, 5123/5125 unsigned), respecting a stride.
fn read(j: &Value, bin: &[u8], i: usize) -> Option<(Vec<f64>, usize)> {
    let a = &j["accessors"][i];
    let bv = &j["bufferViews"][a["bufferView"].as_u64()? as usize];
    let width = match a["type"].as_str()? { "SCALAR" => 1, "VEC2" => 2, "VEC3" => 3, "VEC4" => 4, _ => return None };
    let (ty, size) = match a["componentType"].as_u64()? { 5126 => ('f', 4), 5123 => ('h', 2), 5125 => ('i', 4), _ => return None };
    let start = bv["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
    let stride = bv["byteStride"].as_u64().map_or(width * size, |s| s as usize);
    let mut out = Vec::with_capacity(a["count"].as_u64()? as usize * width);
    for row in 0..a["count"].as_u64()? as usize {
        for c in 0..width {
            let at = start + row * stride + c * size;
            let b = bin.get(at..at + size)?;
            out.push(match ty { 'f' => f32::from_le_bytes(b.try_into().ok()?) as f64, 'h' => u16::from_le_bytes(b.try_into().ok()?) as f64, _ => u32::from_le_bytes(b.try_into().ok()?) as f64 });
        }
    }
    Some((out, width))
}

/// The vertex pairs of every kind's lines, in `KINDS` order.
fn edges(glb: &[u8]) -> Option<[Vec<[f32; 3]>; 4]> {
    let word = |at: usize| Some(u32::from_le_bytes(glb.get(at..at + 4)?.try_into().ok()?) as usize);
    if glb.get(..4)? != b"glTF" { return None; }
    let json_len = word(12)?;
    let j: Value = serde_json::from_slice(glb.get(20..20 + json_len)?).ok()?;
    let bin_at = 20 + json_len;
    let bin = glb.get(bin_at + 8..bin_at + 8 + word(bin_at)?)?;
    let mut lines: [Vec<[f32; 3]>; 4] = Default::default();
    for n in j["nodes"].as_array()? {
        let Some(mi) = n["mesh"].as_u64() else { continue };
        let (m, k) = (node_matrix(n), kind(n["name"].as_str().unwrap_or("")));
        for p in j["meshes"][mi as usize]["primitives"].as_array()? {
            let (pos, _) = read(&j, bin, p["attributes"]["POSITION"].as_u64()? as usize)?;
            let (idx, _) = read(&j, bin, p["indices"].as_u64()? as usize)?;
            let world: Vec<[f32; 3]> = pos.chunks(3).map(|p| {
                let at = |r: usize| m[r][0] * p[0] as f32 + m[r][1] * p[1] as f32 + m[r][2] * p[2] as f32 + m[r][3];
                [at(0), at(1), at(2)]
            }).collect();
            feature_edges(&world, &idx, KINDS[k].2, &mut lines[k]);
        }
    }
    Some(lines.map(|l| simplify(&l, 0.0015, 0.003)))
}

fn pack(lines: &[Vec<[f32; 3]>; 4]) -> Vec<u8> {
    let mut out = b"CARW".to_vec();
    out.extend((lines.len() as u32).to_le_bytes());
    for (k, l) in lines.iter().enumerate() {
        out.extend((l.len() as u32).to_le_bytes());
        out.extend(KINDS[k].1.to_le_bytes());
        for c in KINDS[k].0 { out.extend(c.to_le_bytes()); }
    }
    for p in lines.iter().flatten() {
        for c in [p[0], p[1], p[2], 0.0] { out.extend(((c * 10_000.0).round().clamp(-32767.0, 32767.0) as i16).to_le_bytes()); }
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/car");
    let lines = edges(&std::fs::read(dir.join("car_hologram.glb"))?).ok_or("car_hologram.glb does not parse")?;
    let bytes = pack(&lines);
    std::fs::write(dir.join("car_wire.pack"), &bytes)?;
    println!("{} line segments ({:?} per kind: interior, tyres and wheels, body, lamps), {} bytes", lines.iter().map(|l| l.len() / 2).sum::<usize>(), lines.iter().map(|l| l.len() / 2).collect::<Vec<_>>(), bytes.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLB: &[u8] = include_bytes!("../../assets/car/car_hologram.glb");

    #[test]
    fn the_model_gives_edges_inside_the_cars_size() {
        let lines = edges(GLB).expect("glb parses");
        assert!(lines.iter().all(|l| !l.is_empty() && l.len() % 2 == 0));
        let all = || lines.iter().flatten();
        let span = |k: usize| all().map(|p| p[k]).fold(f32::MIN, f32::max) - all().map(|p| p[k]).fold(f32::MAX, f32::min);
        // the whole model measures 2.40 wide (X), 1.72 high (Y), 5.65 long (Z) (numpy, from the node transforms); creases need not
        // touch the extremes of smooth panels, so: never larger, and within 3 % (measured: 2.396, 1.679, 5.634)
        for (k, full) in [2.40f32, 1.72, 5.65].into_iter().enumerate() { assert!(span(k) <= full + 0.01 && span(k) > full * 0.97, "axis {k}: {} of {full}", span(k)); }
    }

    #[test]
    fn collinear_runs_merge_duplicates_go_and_a_bend_stays() {
        let seg = |a: f32, b: f32| [[a, 0.0, 0.0], [b, 0.0, 0.0]];
        let straight: Vec<[f32; 3]> = [seg(0.0, 1.0), seg(1.0, 2.0), seg(2.0, 3.0), seg(2.0, 3.0)].concat();
        assert_eq!(simplify(&straight, 0.0015, 0.003), [[0.0, 0.0, 0.0], [3.0, 0.0, 0.0]]);
        let bent = [seg(0.0, 1.0), [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0]]].concat();
        assert_eq!(simplify(&bent, 0.0015, 0.003).len(), 4);
        assert!(simplify(&seg(0.0, 0.001), 0.0015, 0.003).is_empty());
    }

    #[test]
    fn the_pack_header_adds_up() {
        let lines = edges(GLB).unwrap();
        let n: usize = lines.iter().map(|l| l.len()).sum();
        assert_eq!(pack(&lines).len(), 8 + 4 * 20 + n * 8);
    }

    #[test]
    fn parts_are_sorted_into_kinds() {
        assert_eq!((kind("Tire_LF"), kind("L_Headlamp"), kind("Dash_Panel"), kind("Hood")), (1, 3, 0, 2));
    }

    #[test]
    fn a_cube_has_its_twelve_edges_and_a_flat_square_only_its_border() {
        // a unit cube (8 corners, 12 triangles): 12 edges are creases; the 6 face diagonals are flat and must not show
        let c: Vec<[f32; 3]> = (0..8).map(|i| [(i & 1) as f32, (i >> 1 & 1) as f32, (i >> 2 & 1) as f32]).collect();
        let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        let idx: Vec<f64> = quads.iter().flat_map(|q| [q[0], q[1], q[2], q[0], q[2], q[3]]).map(|i| i as f64).collect();
        let mut out = Vec::new();
        feature_edges(&c, &idx, 0.8, &mut out);
        assert_eq!(out.len() / 2, 12);
        let mut flat = Vec::new();
        feature_edges(&c[..4], &[0.0, 1.0, 3.0, 0.0, 3.0, 2.0], 0.8, &mut flat);
        assert_eq!(flat.len() / 2, 4);   // the square's border, not the diagonal
    }
}
