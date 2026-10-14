//! Builds native/trem_audio.c (the FMOD + Steam Audio sound path) into
//! trem_audio.dll next to the release / debug output, when the SDK files are
//! in third_party/ (see tools/get_audio_libs.ps1). The game loads that DLL
//! at run time, so builds without the SDKs (and every other platform) just
//! keep Godot's audio.
use std::path::PathBuf;

#[path = "src/splash_png.rs"]
mod splash_png;
#[path = "src/version_line.rs"]
mod version_line;

/// dec090: the version the title screen shows. `git describe` over the
/// release tags (`v1.0`, `v1.0-46-g2e2c4a1`, `-dirty` when the tree has
/// changes; only `v<digit>` tags count, so the texture pack tag is ignored),
/// the commit hash alone when there is no such tag. Without git (a source zip)
/// the environment's TREM_VERSION / TREM_BUILD_DATE, set by whoever builds a
/// release, and then the Cargo version. TREM_BUILD_DATE is otherwise the date
/// of the commit (reproducible), empty without git.
fn export_version() -> (String, String) {
    let git = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git").args(args).output().ok()?;
        let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
        (out.status.success() && !s.is_empty()).then_some(s)
    };
    // a commit appends to logs/HEAD (also in a worktree); a checkout moves HEAD
    for what in ["HEAD", "logs/HEAD"] {
        if let Some(p) = git(&["rev-parse", "--git-path", what]) {
            println!("cargo::rerun-if-changed={p}");
        }
    }
    println!("cargo::rerun-if-env-changed=TREM_VERSION");
    println!("cargo::rerun-if-env-changed=TREM_BUILD_DATE");
    let env = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
    let version = env("TREM_VERSION")
        .or_else(|| {
            git(&[
                "describe", "--tags", "--match", "v[0-9]*", "--always", "--dirty",
            ])
        })
        .or_else(|| env("CARGO_PKG_VERSION"))
        .unwrap_or_default();
    let date = env("TREM_BUILD_DATE")
        .or_else(|| git(&["log", "-1", "--format=%cs"]))
        .unwrap_or_default();
    println!("cargo::rustc-env=TREM_VERSION={version}");
    println!("cargo::rustc-env=TREM_BUILD_DATE={date}");
    (version, date)
}

/// dec080: godot/splash/boot_splash.png (the project's
/// application/boot_splash/image) = splash/base.png with the title screen's
/// version line, so the first picture names the build. Godot shows the boot
/// splash before the extension loads, hence baked here and not drawn at run
/// time. Without a version the file is base.png as it is. The output is
/// generated (git-ignored) and only rewritten when it changes; a failure is a
/// warning, the build goes on.
fn bake_splash(line: &str) {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    for f in [
        "splash/base.png",
        "splash/glyphs.png",
        "src/splash_png.rs",
        "src/version_line.rs",
    ] {
        println!("cargo::rerun-if-changed={}", manifest.join(f).display());
    }
    let out = manifest.join("../../godot/splash/boot_splash.png");
    let made = (|| -> Result<(), String> {
        let rd = |f: &str| std::fs::read(manifest.join(f)).map_err(|e| format!("{f}: {e}"));
        let png = splash_png::bake(&rd("splash/base.png")?, &rd("splash/glyphs.png")?, line)?;
        if std::fs::read(&out).ok().as_deref() != Some(&png[..]) {
            std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&out, &png).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    if let Err(e) = made {
        println!(
            "cargo::warning=boot splash not written ({}): {e}",
            out.display()
        );
    }
}

fn main() {
    let (version, date) = export_version();
    bake_splash(&version_line::version_line(&version, &date));
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
    if !(fmod_inc.join("fmod.h").exists()
        && fmod_lib.join("fmod_vc.lib").exists()
        && sa_inc.join("phonon.h").exists()
        && sa_lib.join("phonon.lib").exists())
    {
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
        // a PDB beside the DLL: crash offsets resolve to source lines
        .arg("/Zi")
        .arg(format!(
            "/Fd{}",
            profile_dir.join("trem_audio_c.pdb").display()
        ))
        .arg(format!("/I{}", fmod_inc.display()))
        .arg(format!("/I{}", sa_inc.display()))
        .arg(manifest.join("native/trem_audio.c"))
        .arg(format!("/Fo{}\\", out.display()))
        .arg(format!("/Fe{}", dll.display()))
        .arg("/link")
        .arg("/DEBUG")
        // only the PDB's file name in the DLL, not the build folder's path
        .arg("/PDBALTPATH:%_PDB%")
        .arg(fmod_lib.join("fmod_vc.lib"))
        .arg(sa_lib.join("phonon.lib"));
    let st = cmd.status().expect("cl.exe");
    if !st.success() {
        panic!("building trem_audio.dll failed");
    }
}
