//! NVK (Mesa's open Vulkan driver for NVIDIA) on a GPU older than Turing:
//! Forward+ draws the materials almost black there (Godot issue #115941)
//! and runs at a few fps (no reclocking). On Linux / BSD the game then
//! restarts once with the OpenGL renderer (on a hybrid laptop that also
//! lands on the integrated GPU) and writes `OVERRIDE` (project.godot
//! `application/config/project_settings_override`), so later launches start
//! in OpenGL directly. `r_nvkOpenGL 0` switches it off and removes the
//! file; a `--rendering-driver` / `--rendering-method` on the command line
//! is always left alone. Turing and newer on NVK keep Vulkan.

use godot::classes::{FileAccess, Os, ProjectSettings, RenderingServer};
use godot::prelude::*;

pub const CVAR: &str = "r_nvkOpenGL";
pub const OVERRIDE: &str = "user://renderer.cfg";
const OVERRIDE_TEXT: &str = "; written by the game: NVK on an NVIDIA GPU older than Turing (r_nvkOpenGL 0 removes it)\n[rendering]\n\nrenderer/rendering_method=\"gl_compatibility\"\n";

/// Is this adapter NVK or nouveau on an NVIDIA GPU older than Turing? NVK
/// names the chip ("GeForce GT 730M (NVK GK208)": GF / GK / GM / GP / GV
/// are Fermi to Volta), nouveau's OpenGL driver its chipset id in hex
/// ("NV117"; Turing starts at NV160).
pub fn nouveau_pre_turing(adapter: &str) -> bool {
    let lower = adapter.to_ascii_lowercase();
    if let Some(i) = lower.find("(nvk ") {
        let chip = &lower[i + 5..];
        return ["gf", "gk", "gm", "gp", "gv"]
            .iter()
            .any(|p| chip.starts_with(p))
            && chip
                .get(2..)
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()));
    }
    lower.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| {
        (4..=5).contains(&w.len())
            && w.starts_with("nv")
            && u32::from_str_radix(&w[2..], 16).is_ok_and(|n| n < 0x160)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Keep,
    /// restart once in OpenGL and remember it
    Restart,
    /// r_nvkOpenGL 0: remove the remembered choice
    Forget,
}

pub fn decide(
    linux: bool,
    driver: &str,
    adapter: &str,
    enabled: bool,
    user_chose: bool,
    remembered: bool,
) -> Action {
    if !linux {
        Action::Keep
    } else if !enabled {
        if remembered {
            Action::Forget
        } else {
            Action::Keep
        }
    } else if !user_chose && driver.eq_ignore_ascii_case("vulkan") && nouveau_pre_turing(adapter) {
        Action::Restart
    } else {
        Action::Keep
    }
}

/// `r_nvkOpenGL` from the config, or from a `+set r_nvkOpenGL N` on the
/// command line (the startup commands run after this check).
pub fn enabled(cvar: i32, startup: &[String]) -> bool {
    let mut on = cvar != 0;
    for l in startup {
        let w: Vec<&str> = l.split_whitespace().collect();
        if w.len() >= 3
            && ["set", "seta", "sets", "setu"].contains(&w[0].to_ascii_lowercase().as_str())
            && w[1].eq_ignore_ascii_case(CVAR)
        {
            on = w[2]
                .trim_matches('"')
                .parse::<i32>()
                .map_or(true, |v| v != 0);
        }
    }
    on
}

/// At startup, once the config is read: true when the game restarts (the
/// caller quits instead of loading).
pub fn check(enabled: bool) -> bool {
    let mut os = Os::singleton();
    // the whole command line: get_cmdline_args() leaves out the options the
    // engine took (--rendering-driver, --path, --resolution, ...)
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        args = os
            .get_cmdline_args()
            .as_slice()
            .iter()
            .chain(os.get_cmdline_user_args().as_slice())
            .map(|a| a.to_string())
            .collect();
    }
    let user_chose = args
        .iter()
        .any(|a| a.starts_with("--rendering-driver") || a.starts_with("--rendering-method"));
    let rs = RenderingServer::singleton();
    let driver = rs.get_current_rendering_driver_name().to_string();
    let adapter = rs.get_video_adapter_name().to_string();
    let path = ProjectSettings::singleton()
        .globalize_path(OVERRIDE)
        .to_string();
    match decide(
        os.has_feature("linuxbsd"),
        &driver,
        &adapter,
        enabled,
        user_chose,
        FileAccess::file_exists(OVERRIDE),
    ) {
        Action::Keep => false,
        Action::Forget => {
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    godot_print!("trem: {CVAR} 0: removed {OVERRIDE}, the next start uses the default renderer");
                }
                Err(e) => {
                    godot_warn!("trem: {CVAR} 0: cannot remove {OVERRIDE}: {e}");
                }
            }
            false
        }
        Action::Restart => {
            if let Err(e) = std::fs::write(&path, OVERRIDE_TEXT) {
                godot_warn!("trem: cannot write {OVERRIDE} ({e}): OpenGL for this run only");
            }
            let mut a = PackedStringArray::new();
            a.push("--rendering-driver");
            a.push("opengl3");
            for x in &args {
                a.push(x.as_str());
            }
            os.set_restart_on_exit_ex(true).arguments(&a).done();
            godot_print!("trem: {adapter} is NVK on an NVIDIA GPU older than Turing (dark materials, a few fps): restarting with the OpenGL renderer ({CVAR} 0 turns this off)");
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_switch_from_config_or_command_line() {
        assert!(enabled(1, &[]));
        assert!(!enabled(0, &[]));
        assert!(!enabled(1, &["set r_nvkOpenGL 0".to_string()]));
        assert!(!enabled(1, &["seta R_NVKOPENGL \"0\"".to_string()]));
        assert!(enabled(0, &["set r_nvkOpenGL 1".to_string()]));
        assert!(enabled(
            1,
            &[
                "set r_enhanced 0".to_string(),
                "connect 1.2.3.4".to_string()
            ]
        ));
    }

    #[test]
    fn pre_turing_names() {
        for n in [
            "GeForce GT 730M (NVK GK208)",
            "NVIDIA GeForce GTX 970 (NVK GM204)",
            "NVIDIA GeForce GTX 1080 (NVK GP104)",
            "NVIDIA TITAN V (NVK GV100)",
            "NV108",
            "NV117",
            "NVE7",
            "nouveau NV137",
        ] {
            assert!(nouveau_pre_turing(n), "{n}");
        }
        for n in [
            "NVIDIA RTX A2000 Laptop GPU (NVK GA107)",
            "NVIDIA GeForce RTX 2060 (NVK TU106)",
            "NVIDIA GeForce RTX 4090 (NVK AD102)",
            "NVIDIA GeForce RTX 5080 (NVK GB203)",
            "NV162",
            "NV194",
            "NVIDIA GeForce GT 730M",
            "NVIDIA GeForce GTX 1080",
            "Intel(R) HD Graphics 4600 (HSW GT2)",
            "Mesa Intel(R) HD Graphics 4600 (HSW GT2)",
            "llvmpipe (LLVM 20.1.2, 256 bits)",
            "AMD Radeon RX 6600 (RADV NAVI23)",
            "",
            "(NVK ",
            "(NVK GK)",
        ] {
            assert!(!nouveau_pre_turing(n), "{n}");
        }
    }

    #[test]
    fn restarts_only_on_linux_vulkan_pre_turing_nvk_unless_told_not_to() {
        let kepler = "GeForce GT 730M (NVK GK208)";
        assert_eq!(
            decide(true, "vulkan", kepler, true, false, false),
            Action::Restart
        );
        assert_eq!(
            decide(true, "vulkan", kepler, true, false, true),
            Action::Restart
        );
        // already OpenGL (the restart, or the remembered choice): never again
        assert_eq!(
            decide(
                true,
                "opengl3",
                "Mesa Intel(R) HD Graphics 4600 (HSW GT2)",
                true,
                false,
                true
            ),
            Action::Keep
        );
        assert_eq!(
            decide(true, "opengl3", "NV108", true, false, false),
            Action::Keep
        );
        // --rendering-driver / --rendering-method on the command line
        assert_eq!(
            decide(true, "vulkan", kepler, true, true, false),
            Action::Keep
        );
        // the off-switch, and removing the remembered choice
        assert_eq!(
            decide(true, "vulkan", kepler, false, false, false),
            Action::Keep
        );
        assert_eq!(
            decide(
                true,
                "opengl3",
                "Mesa Intel(R) HD Graphics 4600 (HSW GT2)",
                false,
                false,
                true
            ),
            Action::Forget
        );
        // other GPUs and platforms
        assert_eq!(
            decide(
                true,
                "vulkan",
                "NVIDIA RTX A2000 Laptop GPU (NVK GA107)",
                true,
                false,
                false
            ),
            Action::Keep
        );
        assert_eq!(
            decide(
                true,
                "vulkan",
                "llvmpipe (LLVM 20.1.2, 256 bits)",
                true,
                false,
                false
            ),
            Action::Keep
        );
        assert_eq!(
            decide(false, "vulkan", kepler, true, false, false),
            Action::Keep
        );
    }
}
