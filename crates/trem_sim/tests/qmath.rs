use trem_sim::qmath::*;

#[test]
fn normalize_matches_q3_semantics() {
    let mut v = [3.0f32, 4.0, 0.0];
    let l = normalize(&mut v);
    assert_eq!(l, 5.0);
    assert_eq!(v, [0.6, 0.8, 0.0]);
    let mut z = [0.0f32; 3];
    assert_eq!(normalize(&mut z), 0.0);
}

#[test]
fn angle_vectors_axes_are_orthonormal() {
    let (f, r, u) = angle_vectors(&[30.0, 45.0, 0.0]);
    for v in [f, r, u] {
        assert!((length(&v) - 1.0).abs() < 1e-5);
    }
    assert!(dot(&f, &r).abs() < 1e-5 && dot(&f, &u).abs() < 1e-5);
}

#[test]
fn vectoangles_roundtrip() {
    let (f, _, _) = angle_vectors(&[-20.0, 135.0, 0.0]);
    let a = vectoangles(&f);
    assert!((a[YAW] - 135.0).abs() < 1e-3, "{a:?}");
    assert!((angle_normalize180(a[PITCH]) + 20.0).abs() < 0.01, "{a:?}"); // AngleNormalize360 quantises to 360/65536
}

#[test]
fn angle2short_wraps() {
    assert_eq!(angle2short_f(360.0), 0);
    assert_eq!(angle2short_f(90.0), 16384);
    assert_eq!(angle2short_f(-90.0), 49152);
}
