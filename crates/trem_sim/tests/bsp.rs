//! IBSP loader: round-trip, collision equivalence with the sheet clip map,
//! and the C-oracle golden trace replayed on the BSP-loaded arena.
use sha2::{Digest, Sha256};
use trem_sim::gen::enums::EntityKind;
use trem_sim::gen::maps::MapId;
use trem_sim::kernel::rng::Rng;
use trem_sim::kernel::world::{EType, World};
use trem_sim::kernel::{bsp, cm};

#[test]
fn ibsp_write_parse_roundtrip() {
    let b = bsp::from_sheet(MapId::Arena);
    let bytes = b.write();
    assert_eq!(&bytes[0..4], b"IBSP");
    let back = bsp::parse(&bytes).expect("parse");
    assert_eq!(back.shaders, b.shaders);
    assert_eq!(back.planes, b.planes);
    assert_eq!(back.brushes, b.brushes);
    assert_eq!(back.surfaces, b.surfaces);
    assert_eq!(back.verts, b.verts);
    assert_eq!(back.entity_list(), b.entity_list());
    assert!(back.surfaces.len() >= 29 * 4, "drawable faces exported");
}

#[test]
fn malformed_bsp_is_rejected() {
    let mut bytes = bsp::from_sheet(MapId::Arena).write();
    assert!(bsp::parse(&bytes[..40]).is_err());
    bytes[4] = 47;
    assert!(bsp::parse(&bytes).unwrap_err().contains("version"));
}

#[test]
fn bsp_clip_map_traces_like_the_sheet_clip_map() {
    let sheet = cm::from_sheet(MapId::Arena);
    let loaded = bsp::parse(&bsp::from_sheet(MapId::Arena).write()).unwrap().clip_map();
    let mut rng = Rng::new(1234);
    let mut r = |s: f32| (rng.crandom() as f32) * s;
    let sizes = [([0.0f32; 3], [0.0f32; 3]), ([-15.0, -15.0, -24.0], [15.0, 15.0, 32.0]), ([-8.0; 3], [8.0; 3])];
    let mut hits = 0;
    for n in 0..3000 {
        let (mins, maxs) = sizes[n % sizes.len()];
        let a = [r(1500.0), r(1000.0), 200.0 + r(260.0)];
        let b = [r(1500.0), r(1000.0), 200.0 + r(260.0)];
        let t1 = sheet.box_trace(&a, &b, Some(&mins), Some(&maxs), -1);
        let t2 = loaded.box_trace(&a, &b, Some(&mins), Some(&maxs), -1);
        assert_eq!((t1.fraction, t1.endpos, t1.startsolid, t1.allsolid, t1.contents), (t2.fraction, t2.endpos, t2.startsolid, t2.allsolid, t2.contents), "trace {n} {a:?}->{b:?}");
        if t1.fraction < 1.0 {
            hits += 1;
            assert_eq!(t1.plane.normal, t2.plane.normal, "trace {n}");
        }
        assert_eq!(sheet.point_contents(&a), loaded.point_contents(&a));
    }
    assert!(hits > 500);
}

#[test]
fn golden_trace_holds_on_the_bsp_loaded_arena() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let want = std::fs::read_to_string(root.join("re/traces/arena.sha256")).unwrap();
    let want = want.split_whitespace().next().unwrap().to_string();
    let clip = bsp::parse(&bsp::from_sheet(MapId::Arena).write()).unwrap().clip_map();
    let got = format!("{:x}", Sha256::digest(trem_sim::parity::dump_scenarios_on(clip, None).as_bytes()));
    assert_eq!(got, want);
}

#[test]
fn world_from_bsp_spawns_structures_and_plays() {
    let b = bsp::parse(&bsp::from_sheet(MapId::Arena).write()).unwrap();
    let mut w = World::from_bsp(&b, "arena-bsp");
    assert!(!w.has_waypoints);
    assert!(w.spots.iter().any(|s| s.kind == EntityKind::PlayerStart));
    let sheet = World::new(MapId::Arena);
    let count = |w: &World| (64..w.num_entities).filter(|&i| w.ents[i].inuse && w.ents[i].etype == EType::Buildable).count();
    assert_eq!(count(&w), count(&sheet));
    for k in 0..2 {
        trem_sim::kernel::bot::add_bot(&mut w, &format!("a{k}"), trem_sim::gen::enums::Team::Aliens);
        trem_sim::kernel::bot::add_bot(&mut w, &format!("h{k}"), trem_sim::gen::enums::Team::Humans);
    }
    for _ in 0..(20_000 / 16) {
        w.frame(16);
    }
    assert!(w.level.num_live_human_clients + w.level.num_live_alien_clients > 0);
}

/// CM_BiSphereTrace / CM_ProximityToBrush (light flare REALFADE): a ray that
/// clears a box by less than the end radius reports a lateral fraction of
/// (closest edge distance / radius)^2; a ray through the box reports 0.
#[test]
fn bisphere_lateral_fraction() {
    use trem_sim::kernel::cm::{bisphere_against_box, distance_between_line_segments_squared};
    let (mn, mx) = ([-16.0f32, -16.0, -16.0], [16.0f32, 16.0, 16.0]);
    // segment past the top edges of the box, ~4 units clear above (the C
    // bounds are inverted on axes where start == end, so z varies a little)
    let (s, e) = ([-100.0f32, 0.0, 20.0], [100.0f32, 0.0, 20.01]);
    let t = bisphere_against_box(&s, &e, 1.0, 1.0, &mn, &mx, &[0.0; 3], 1, 1);
    assert_eq!(t.fraction, 1.0, "small spheres miss");
    let t = bisphere_against_box(&s, &e, 8.0, 8.0, &mn, &mx, &[0.0; 3], 1, 1);
    assert!(t.fraction < 1.0);
    assert!((t.lateral_fraction - 16.0 / 64.0).abs() < 2e-3, "lateral {}", t.lateral_fraction);
    // through the box
    let (s, e) = ([-100.0f32, 0.0, 0.0], [100.0f32, 0.0, 0.01]);
    let t = bisphere_against_box(&s, &e, 8.0, 8.0, &mn, &mx, &[0.0; 3], 1, 1);
    assert_eq!(t.lateral_fraction, 0.0);
    let (d, s0, t0) = distance_between_line_segments_squared(&[0.0, 0.0, 0.0], &[10.0, 0.0, 0.0], &[5.0, -5.0, 3.0], &[5.0, 5.0, 3.0]);
    assert!((d - 9.0).abs() < 1e-4 && (s0 - 0.5).abs() < 1e-4 && (t0 - 0.5).abs() < 1e-4);
}
