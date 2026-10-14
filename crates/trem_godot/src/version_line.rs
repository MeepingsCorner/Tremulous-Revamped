//! The version text the title screen and the boot splash show (dec077,
//! dec080). Plain Rust without dependencies: build.rs includes this
//! file too, so the splash it bakes and the menu show the same line.

/// `version` is build.rs's TREM_VERSION (`git describe` over the `v<digit>`
/// tags, `v1.0` / `v1.0-12-gabc1234`, the commit hash alone while there is no
/// such tag; or the environment / Cargo version), `date` its TREM_BUILD_DATE.
/// A bare Cargo version (`0.1.0`) gets a `v`. Empty when there is no version,
/// so the caller shows nothing rather than a placeholder.
pub fn version_line(version: &str, date: &str) -> String {
    let v = version.trim();
    if v.is_empty() {
        return String::new();
    }
    let v = if v.contains('.') && v.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("v{v}")
    } else {
        v.to_string()
    };
    let d = date.trim();
    if d.is_empty() {
        v
    } else {
        format!("{v} - {d}")
    }
}

#[cfg(test)]
mod tests {
    use super::version_line;

    #[test]
    fn version_line_formats() {
        assert_eq!(version_line("v1.0", "2026-10-14"), "v1.0 - 2026-10-14");
        assert_eq!(
            version_line("v1.0-12-gabc1234", "2026-10-14"),
            "v1.0-12-gabc1234 - 2026-10-14"
        );
        assert_eq!(
            version_line("fd98a14", "2026-10-14"),
            "fd98a14 - 2026-10-14"
        );
        assert_eq!(version_line("0.1.0", ""), "v0.1.0");
        assert_eq!(version_line("", "2026-10-14"), "");
        assert_eq!(version_line("", ""), "");
    }
}
