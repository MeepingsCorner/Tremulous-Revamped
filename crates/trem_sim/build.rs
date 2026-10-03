// Thin shim (MDD §5.6): preflight gates every build (D5), then the emitter
// writes one module per sheet into $OUT_DIR (§5.9).
fn main() {
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../..").canonicalize().unwrap();
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let cfg = sheetty::Config {
        sheets: root.join("sheets"),
        root: root.clone(),
        out: Some(out),
        strict: std::env::var("SHEETTY_STRICT").is_ok(),
        fn_sig_path: "crate::kernel::sig".into(),
        kernel_dir: Some(manifest.join("src/kernel")),
        emit_cargo_hints: true,
    };
    match sheetty::run(&cfg) {
        Ok(o) => {
            if o.diags.warnings() > 0 {
                println!("cargo::warning=sheetty: {} warning(s), {} unimplemented plan row(s); run `sheetty preflight`", o.diags.warnings(), o.coverage.unimplemented.len());
            }
        }
        Err(d) => {
            eprintln!("{}", d.human("PREFLIGHT FAILED (build gated by D5)"));
            eprintln!("{}", d.json());
            std::process::exit(1);
        }
    }
}
