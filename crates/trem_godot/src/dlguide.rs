//! The download guide: what the player sees instead of the dead end when
//! the computer being joined refuses file downloads and a required pk3 is
//! missing ("You must install these files manually: ..."). Two layouts:
//! A1 when its server info carries an `sv_dlURL` (a button opens that
//! address for the file), B2 when it says nothing (a button opens the first
//! community source of `cl_downloadSources`). Both open the exact folder the
//! file belongs in, show its path, and offer Join again / Disconnect.
//!
//! This file holds the parts that need no Godot (name and URL checks, the
//! choice of layout, the texts and the two menus) so they can be unit tested.
//! Everything the player can read here avoids the word "server" on purpose.

/// `cl_downloadSources`: the community sites the B2 layout may open, space
/// separated, first usable one wins. The one place to change them: edit the
/// default below, or set the cvar (`+set cl_downloadSources "http://a/ http://b/"`).
/// Only http and https addresses are used.
pub const DEFAULT_SOURCES: &str = "http://dl.zittrig.eu/base/";

/// The menu files this module contributes (loaded with the port's pages).
pub const MENU_FILE: &str = "ui/port_download.menu";
pub const MENU_A: &str = "download_guide_a";
pub const MENU_B: &str = "download_guide_b";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// the server info has a usable `sv_dlURL`
    A,
    /// nothing usable: a community source
    B,
}

/// What the guide shows and opens, worked out once when the prompt appears.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub tier: Tier,
    /// the first missing pk3's name ("map-karith-1.1.0.pk3")
    pub file: String,
    /// the folder under user:// it belongs in ("base", "slacker")
    pub folder: String,
    /// how many more missing files follow this one
    pub more: usize,
    /// the address the first button opens
    pub url: Option<String>,
    /// the address' host (and port), shown on the A1 button
    pub domain: String,
}

/// A pk3 path as the guide may use it in an address or a folder name:
/// only `[A-Za-z0-9_./+-]`, no `..`, no leading `/`, no empty parts, ends
/// in `.pk3`.
pub fn valid_pk3_path(name: &str) -> bool {
    if name.is_empty() || name.starts_with('/') || name.contains("..") {
        return false;
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b'+' | b'-'))
    {
        return false;
    }
    if name.split('/').any(|p| p.is_empty()) {
        return false;
    }
    name.len() > 4 && name[name.len() - 4..].eq_ignore_ascii_case(".pk3")
}

/// The host (and port) of an http or https address, or None when it is not
/// one the guide will show or open: other schemes, a user name in front of
/// the host (`http://a.example@b.example/`), a query or fragment, spaces,
/// control characters, quotes, backslashes or anything not ASCII.
pub fn http_authority(url: &str) -> Option<&str> {
    let rest = ["http://", "https://"].iter().find_map(|p| {
        url.get(..p.len())
            .filter(|h| h.eq_ignore_ascii_case(p))
            .map(|_| &url[p.len()..])
    })?;
    if url.len() > 300
        || !url.bytes().all(|b| b.is_ascii_graphic())
        || url.bytes().any(|b| {
            matches!(
                b,
                b'"' | b'\\' | b'?' | b'#' | b'<' | b'>' | b'`' | b'^' | b'|' | b'{' | b'}'
            )
        })
    {
        return None;
    }
    let auth = rest.split('/').next().unwrap_or("");
    if auth.is_empty()
        || auth.contains('@')
        || !auth
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'))
    {
        return None;
    }
    Some(auth)
}

pub fn is_https(url: &str) -> bool {
    url.get(..8)
        .is_some_and(|h| h.eq_ignore_ascii_case("https://"))
}

/// `sv_dlURL` + "/" + the pk3 path, or None unless both pass the checks.
pub fn download_url(base: &str, path: &str) -> Option<String> {
    http_authority(base)?;
    if !valid_pk3_path(path) {
        return None;
    }
    Some(format!("{}/{}", base.trim_end_matches('/'), path))
}

/// The first usable address of `cl_downloadSources` (empty: the default).
pub fn first_source(list: &str) -> Option<String> {
    let list = if list.trim().is_empty() {
        DEFAULT_SOURCES
    } else {
        list
    };
    list.split_whitespace()
        .find(|s| http_authority(s).is_some())
        .map(|s| s.to_string())
}

/// Tier and addresses for the missing files (their names as the download
/// list gives them, "<folder>/<name>.pk3"). None when no name passes the
/// checks: the old prompt is shown then.
pub fn plan(files: &[String], sv_dl_url: &str, sources: &str) -> Option<Plan> {
    let ok: Vec<&String> = files.iter().filter(|f| valid_pk3_path(f)).collect();
    let first = ok.first()?;
    let (folder, file) = match first.rsplit_once('/') {
        Some((d, f)) => (d.to_string(), f.to_string()),
        None => ("base".to_string(), first.to_string()),
    };
    let more = ok.len() - 1;
    if let Some(url) = download_url(sv_dl_url, first) {
        let domain = http_authority(&url).unwrap_or("").to_string();
        return Some(Plan {
            tier: Tier::A,
            file,
            folder,
            more,
            url: Some(url),
            domain,
        });
    }
    let url = first_source(sources);
    let domain = url
        .as_deref()
        .and_then(http_authority)
        .unwrap_or("")
        .to_string();
    Some(Plan {
        tier: Tier::B,
        file,
        folder,
        more,
        url,
        domain,
    })
}

/// The folder's path as the player sees it: `user_dir` is Godot's globalized
/// `user://` (forward slashes), the result ends in a separator.
pub fn display_path(user_dir: &str, folder: &str, windows: bool) -> String {
    let p = format!("{}/{}/", user_dir.trim_end_matches('/'), folder);
    if windows {
        p.replace('/', "\\")
    } else {
        p
    }
}

impl Plan {
    pub fn menu(&self) -> &'static str {
        match self.tier {
            Tier::A => MENU_A,
            Tier::B => MENU_B,
        }
    }

    /// The menus' texts: (lower-case ui cvar, value). `path` is `display_path`.
    pub fn cvars(&self, path: &str) -> Vec<(&'static str, String)> {
        let trust = "The game reads these files: use only sites you trust.";
        let more = match self.more {
            0 => String::new(),
            1 => " One more file is needed after this one.".to_string(),
            n => format!(" {n} more files are needed after this one."),
        };
        let url = self.url.clone().unwrap_or_default();
        let note = if is_https(&url) {
            trust.to_string()
        } else {
            format!("Not encrypted (http). {trust}")
        };
        vec![
            ("ui_dg_intro_a", INTRO_A.to_string()),
            ("ui_dg_intro_b", INTRO_B.to_string()),
            ("ui_dg_hint", HINT_B.to_string()),
            ("ui_dg_open", format!("Open {} in browser", self.domain)),
            ("ui_dg_url", url),
            ("ui_dg_note", note),
            (
                "ui_dg_save",
                format!("Save {} in this folder:{more}", self.file),
            ),
            ("ui_dg_path", path.to_string()),
            (
                "ui_dg_file",
                format!(
                    "{}{}",
                    self.file,
                    if self.more > 0 {
                        format!(" (+{})", self.more)
                    } else {
                        String::new()
                    }
                ),
            ),
            (
                "ui_dg_online",
                if self.url.is_some() {
                    format!("Look for {} online", self.file)
                } else {
                    String::new()
                },
            ),
        ]
    }
}

/// The text of the guide's menus (`ui/port_download.menu`). Numbers, not
/// ui/menudef.h names, like the port's other pages; the texts come from the
/// `ui_dg_*` cvars the plan fills.
pub fn menu_text() -> String {
    let frame = |name: &str, w: i32, h: i32| {
        format!(
            r#"    itemDef
    {{
      name frame
      style 1
      rect 0 0 {w} {h}
      backcolor 0 0 0 1
      visible 1
      decoration
      border 1
      borderSize 1.0
      borderColor 0.5 0.5 0.5 1
    }}
    itemDef
    {{
      name title
      type 1
      text "Install this file yourself"
      style 0
      rect 0 8 {w} 22
      textalign 1
      textvalign 1
      textscale .33
      forecolor 1 1 1 1
      visible 1
      decoration
    }}
"#
        ) + &format!("    // {name}\n")
    };
    // a line of text from a cvar (wrapped within its rect)
    let text =
        |name: &str, cvar: &str, x: i32, y: i32, w: i32, h: i32, scale: &str, color: &str| {
            format!(
                r#"    itemDef
    {{
      name {name}
      type 1
      cvar "{cvar}"
      style 0
      rect {x} {y} {w} {h}
      textalign 0
      textvalign 2
      textscale {scale}
      forecolor {color}
      wrapped
      visible 1
      decoration
    }}
"#
            )
        };
    // a button: fixed text, or the text of a cvar
    let button = |name: &str, label: &str, x: i32, y: i32, w: i32, scale: &str, action: &str| {
        let label = match label.strip_prefix('$') {
            Some(c) => format!("cvar \"{c}\""),
            None => format!("text \"{label}\""),
        };
        // wide buttons and the left column read from the left, the rest are centred
        let (align, alignx) = if w >= 200 || x == 36 { (0, 6) } else { (1, 0) };
        format!(
            r#"    itemDef
    {{
      name {name}
      type 2
      {label}
      style 0
      rect {x} {y} {w} 18
      textalign {align}
      textalignx {alignx}
      textvalign 1
      textscale {scale}
      forecolor 1 1 1 1
      visible 1
      action
      {{
        play "sound/misc/menu1.wav";
        {action}
      }}
    }}
"#
        )
    };
    let step = |n: &str, y: i32| {
        format!(
            r#"    itemDef
    {{
      name step{n}
      type 1
      text "{n}"
      style 0
      rect 12 {y} 16 18
      textalign 1
      textvalign 1
      textscale .27
      forecolor 1 .75 0 1
      visible 1
      decoration
    }}
"#
        )
    };
    let hdr = |name: &str, x: i32, y: i32, w: i32, h: i32| {
        format!(
            r#"  menuDef
  {{
    name "{name}"
    visible 0
    fullScreen 0
    rect {x} {y} {w} {h}
    focusColor 1 .75 0 1
    onESC {{ close {name}; uiScript downloadIgnore }}

"#
        )
    };
    let online = "exec \"dlguide_online\"";
    let folder = "exec \"dlguide_folder\"";
    let join = |m: &str| format!("close {m}; exec \"reconnect\"");
    let leave = |m: &str| format!("close {m}; exec \"disconnect\"");

    let mut s = String::from("// the download guide (dlguide.rs): numbers, not ui/menudef.h names (type 1 text, 2 button)\n{\n");
    // A1: the address comes with the file's name
    s += &hdr(MENU_A, 145, 50, 350, 280);
    s += &frame("A1", 350, 280);
    s += &text(
        "intro",
        "ui_dg_intro_a",
        18,
        32,
        314,
        34,
        ".22",
        ".7 .7 .7 1",
    );
    s += &step("1", 70);
    s += &button("open", "$ui_dg_open", 36, 70, 296, ".27", online);
    s += &text("url", "ui_dg_url", 40, 90, 292, 12, ".19", ".75 .85 1 1");
    s += &text("note", "ui_dg_note", 40, 103, 292, 26, ".19", ".7 .7 .7 1");
    s += &step("2", 134);
    s += &button(
        "folder",
        "Open the game's folder on this computer",
        36,
        134,
        296,
        ".27",
        folder,
    );
    s += &text("save", "ui_dg_save", 40, 154, 292, 26, ".19", ".7 .7 .7 1");
    s += &text("path", "ui_dg_path", 40, 182, 292, 30, ".19", ".75 .85 1 1");
    s += &step("3", 216);
    s += &button("join", "Join again", 36, 216, 296, ".27", &join(MENU_A));
    s += &button("leave", "Disconnect", 36, 248, 120, ".25", &leave(MENU_A));
    s += "  }\n";
    // B2: nothing is known about where the file is
    s += &hdr(MENU_B, 165, 55, 310, 290);
    s += &frame("B2", 310, 290);
    s += &text("file", "ui_dg_file", 18, 32, 274, 14, ".25", "1 .75 0 1");
    s += &text(
        "intro",
        "ui_dg_intro_b",
        18,
        50,
        274,
        40,
        ".22",
        ".7 .7 .7 1",
    );
    s += &button("online", "$ui_dg_online", 18, 94, 274, ".25", online);
    s += &text("hint", "ui_dg_hint", 18, 114, 274, 40, ".19", ".7 .7 .7 1");
    s += &button(
        "folder",
        "Open the game's folder on this computer",
        18,
        158,
        274,
        ".25",
        folder,
    );
    s += &text("path", "ui_dg_path", 18, 178, 274, 36, ".19", ".75 .85 1 1");
    s += &button("join", "Join again", 18, 250, 130, ".25", &join(MENU_B));
    s += &button("leave", "Disconnect", 162, 250, 130, ".25", &leave(MENU_B));
    s += "  }\n}\n";
    s
}

/// Intro texts that do not depend on the plan.
pub const INTRO_A: &str = "The computer you are joining does not send files, but it says where to get this one. Three steps:";
pub const INTRO_B: &str =
    "The computer you are joining does not send files and does not say where to get them.";
pub const HINT_B: &str = "A community site, not run by this game; it may not have this file. The game reads these files: use sites you trust.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pk3_names_are_checked() {
        assert!(valid_pk3_path("base/map-karith-1.1.0.pk3"));
        assert!(valid_pk3_path("slacker/a_b+c.PK3"));
        for bad in [
            "",
            "/base/x.pk3",
            "base/../x.pk3",
            "base/x.pk",
            "base/x.pk3.exe",
            "base\\x.pk3",
            "base/x y.pk3",
            "base//x.pk3",
            "ba@se/x.pk3",
            "base/x.pk3\n",
            ".pk3",
            "base/é.pk3",
            "base/x%2e.pk3",
        ] {
            assert!(!valid_pk3_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn urls_are_http_or_https_only() {
        assert_eq!(
            download_url("http://dl.example.org/base/", "base/a.pk3").as_deref(),
            Some("http://dl.example.org/base/base/a.pk3")
        );
        assert_eq!(
            download_url("https://dl.example.org:8080", "base/a.pk3").as_deref(),
            Some("https://dl.example.org:8080/base/a.pk3")
        );
        for bad in [
            "ftp://x.org",
            "file:///etc",
            "javascript:alert(1)",
            "http://",
            "http:///x",
            "http://a.org@evil.org/",
            "http://a.org/x?y=1",
            "http://a.org/ x",
            "http://a b.org/",
            "dl.example.org",
            "http://a.org/\"",
            "http://ä.org/",
        ] {
            assert_eq!(download_url(bad, "base/a.pk3"), None, "{bad:?}");
        }
        assert_eq!(download_url("http://dl.example.org", "../a.pk3"), None);
        assert!(download_url("HTTPS://DL.example.org", "base/a.pk3").is_some());
    }

    #[test]
    fn tier_choice() {
        let files = vec![
            "base/map-a-1.0.pk3".to_string(),
            "base/map-b.pk3".to_string(),
        ];
        let a = plan(&files, "http://dl.example.org/maps", DEFAULT_SOURCES).unwrap();
        assert_eq!(
            (
                a.tier,
                a.file.as_str(),
                a.folder.as_str(),
                a.more,
                a.domain.as_str()
            ),
            (Tier::A, "map-a-1.0.pk3", "base", 1, "dl.example.org")
        );
        assert_eq!(
            a.url.as_deref(),
            Some("http://dl.example.org/maps/base/map-a-1.0.pk3")
        );
        // nothing, or something that is not an address: the community source
        for none in ["", "ftp://x.org/", "javascript:1"] {
            let b = plan(&files, none, DEFAULT_SOURCES).unwrap();
            assert_eq!(
                (b.tier, b.url.as_deref(), b.domain.as_str()),
                (Tier::B, Some("http://dl.zittrig.eu/base/"), "dl.zittrig.eu")
            );
        }
        // no usable source: B without a button address
        assert_eq!(plan(&files, "", "ftp://x.org").unwrap().url, None);
        // a mod's folder; names that fail the check are left out; none left: the old prompt
        assert_eq!(
            plan(&["slacker/x.pk3".into()], "", "ftp://x.org")
                .unwrap()
                .folder,
            "slacker"
        );
        assert_eq!(
            plan(&["../x.pk3".into(), "base/y.pk3".into()], "", "")
                .unwrap()
                .file,
            "y.pk3"
        );
        assert_eq!(plan(&["../x.pk3".into()], "http://a.org", ""), None);
    }

    #[test]
    fn source_list() {
        assert_eq!(
            first_source(DEFAULT_SOURCES).as_deref(),
            Some("http://dl.zittrig.eu/base/")
        );
        assert_eq!(
            first_source("ftp://a.org  https://b.org/x/ http://c.org/").as_deref(),
            Some("https://b.org/x/")
        );
        assert_eq!(
            first_source("").as_deref(),
            Some("http://dl.zittrig.eu/base/")
        );
        assert_eq!(first_source("ftp://a.org"), None);
    }

    #[test]
    fn path_and_texts_say_computer_not_server() {
        assert_eq!(display_path("C:/Users/Player/AppData/Roaming/Godot/app_userdata/Tremulous (Godot + Rust port)", "base", true), "C:\\Users\\Player\\AppData\\Roaming\\Godot\\app_userdata\\Tremulous (Godot + Rust port)\\base\\");
        assert_eq!(
            display_path("/data/p/app_userdata/T/", "slacker", false),
            "/data/p/app_userdata/T/slacker/"
        );
        let a = plan(
            &["base/map-a.pk3".into(), "base/b.pk3".into()],
            "http://dl.example.org",
            "",
        )
        .unwrap();
        let mut all: Vec<String> = a.cvars("/p/").into_iter().map(|(_, v)| v).collect();
        let b = plan(&["base/map-a.pk3".into()], "", DEFAULT_SOURCES).unwrap();
        all.extend(b.cvars("/p/").into_iter().map(|(_, v)| v));
        all.extend([INTRO_A, INTRO_B, HINT_B, &menu_text()].map(String::from));
        assert!(all
            .iter()
            .all(|t| !t.to_ascii_lowercase().contains("server")));
        let cv = a.cvars("/p/");
        assert!(cv.contains(&("ui_dg_open", "Open dl.example.org in browser".to_string())));
        assert!(cv.contains(&(
            "ui_dg_note",
            "Not encrypted (http). The game reads these files: use only sites you trust."
                .to_string()
        )));
        assert!(b
            .cvars("/p/")
            .contains(&("ui_dg_online", "Look for map-a.pk3 online".to_string())));
    }
}
