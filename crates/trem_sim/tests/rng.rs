//! MDD §7.5 "RNG is the crux": the port's rand() must equal the oracle's.
//! Expected values printed by re/harness/shims.c after srand(0x5eed1234).
#[test]
fn rand_stream_matches_oracle() {
    let mut r = trem_sim::rng::Rng::default();
    let got: Vec<i32> = (0..5).map(|_| r.rand()).collect();
    assert_eq!(got, EXPECTED);
}
const EXPECTED: [i32; 5] = [2002412590, 645524201, 1622285137, 1118932688, 1140672172];
