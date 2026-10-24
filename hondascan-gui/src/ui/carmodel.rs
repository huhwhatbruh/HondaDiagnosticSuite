//! The hologram car's wires: `assets/car/car_wire.pack`, baked from the owner's model by `src/bin/bakecar.rs` (which also
//! documents the format). Line segments in metres (Y up, nose along +Z, the driver's left at +X), in four groups drawn in
//! this order: interior, tyres and wheels, body, lamps, each with its brightness. The pack also stores a colour per group; the
//! turntable uses its own palette, so it is not read.

pub const PACK: &[u8] = include_bytes!("../../assets/car/car_wire.pack");

/// A run of `count` vertices from `start` (consecutive pairs are segments).
pub struct Group { pub start: i32, pub count: i32, pub strength: f32 }

/// The vertices as 4 x i16 each (0.1 mm units, last always 0) and the groups.
pub struct Wire<'a> { pub verts: &'a [u8], pub groups: Vec<Group> }

/// metres per stored unit
pub const UNIT: f32 = 1.0 / 10_000.0;

pub fn parse(pack: &[u8]) -> Option<Wire<'_>> {
    let word = |at: usize| Some(u32::from_le_bytes(pack.get(at..at + 4)?.try_into().ok()?));
    if pack.get(..4)? != b"CARW" { return None; }
    let n = word(4)? as usize;
    let mut groups = Vec::with_capacity(n);
    let mut start = 0;
    for g in 0..n {
        let at = 8 + g * 20;
        let f = |o: usize| word(at + o).map(f32::from_bits);
        let count = word(at)? as i32;
        groups.push(Group { start, count, strength: f(4)? });
        start += count;
    }
    let verts = pack.get(8 + n * 20..)?;
    (verts.len() == start as usize * 8).then_some(Wire { verts, groups })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_pack_parses_and_every_group_has_lines() {
        let w = parse(PACK).expect("pack parses");
        assert_eq!(w.groups.len(), 4);
        assert!(w.groups.iter().all(|g| g.count > 0 && g.count % 2 == 0 && g.strength > 0.0));
        assert_eq!(w.groups.iter().map(|g| g.count as usize).sum::<usize>() * 8, w.verts.len());
    }

    #[test]
    fn a_truncated_or_foreign_pack_is_refused() {
        assert!(parse(&PACK[..PACK.len() - 1]).is_none());
        assert!(parse(b"GLTF\0\0\0\0").is_none());
    }
}
