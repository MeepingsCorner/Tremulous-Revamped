//! Builds native/trem_audio.c (the FMOD + Steam Audio sound path) into
//! trem_audio.dll next to the release / debug output, when the SDK files are
//! in third_party/ (see tools/get_audio_libs.ps1). The game loads that DLL
//! at run time, so builds without the SDKs (and every other platform) just
//! keep Godot's audio.
use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-changed=native/trem_audio.c");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../..");
    let fmod_inc = root.join("third_party/fmod/inc");
    let fmod_lib = root.join("third_party/fmod/lib");
    let sa_inc = root.join("third_party/steamaudio/include");
    let sa_lib = root.join("third_party/steamaudio/lib");
    for p in [&fmod_inc, &fmod_lib, &sa_inc, &sa_lib] {
        println!("cargo::rerun-if-changed={}", p.display());
    }
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "windows" || env != "msvc" {
        return;
    }
    if !(fmod_inc.join("fmod.h").exists() && fmod_lib.join("fmod_vc.lib").exists() && sa_inc.join("phonon.h").exists() && sa_lib.join("phonon.lib").exists()) {
        println!("cargo::warning=trem_audio.dll not built: FMOD / Steam Audio SDK files missing in third_party (run tools/get_audio_libs.ps1)");
        return;
    }
    // OUT_DIR = target/<profile>/build/trem_godot-*/out -> target/<profile>
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let profile_dir = out.ancestors().nth(3).unwrap().to_path_buf();
    let dll = profile_dir.join("trem_audio.dll");
    let compiler = cc::Build::new().opt_level(2).get_compiler();
    let mut cmd = compiler.to_command();
    cmd.arg("/nologo")
        .arg("/LD")
        .arg("/O2")
        .arg("/W3")
        .arg(format!("/I{}", fmod_inc.display()))
        .arg(format!("/I{}", sa_inc.display()))
        .arg(manifest.join("native/trem_audio.c"))
        .arg(format!("/Fo{}\\", out.display()))
        .arg(format!("/Fe{}", dll.display()))
        .arg("/link")
        .arg(fmod_lib.join("fmod_vc.lib"))
        .arg(sa_lib.join("phonon.lib"));
    let st = cmd.status().expect("cl.exe");
    if !st.success() {
        panic!("building trem_audio.dll failed");
    }
}
