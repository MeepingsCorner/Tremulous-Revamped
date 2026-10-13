//! Windows Firewall check for a hosted game (port addition, dec047).
//!
//! A listen server on Windows is reachable from its own PC whatever the
//! firewall says (traffic to the PC's own address is not filtered), so a
//! test on one PC passes while a friend on the same Wi-Fi sees nothing:
//! with no inbound allow rule for the game's exe, Windows drops every
//! packet from other PCs, the LAN scan's answers included. This asks
//! Windows (PowerShell, read only, on a thread) which inbound rules name
//! the running exe and which network categories are active, and tells the
//! host in one line. `firewall_allow` adds the rule through Windows' own
//! administrator prompt; the player approves or declines it there.

use std::process::Command;

/// What the firewall does with packets from other PCs for this exe.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// every active network category has an enabled allow rule and no block
    Allowed(Vec<String>),
    /// an enabled block rule covers an active category
    Blocked(Vec<String>),
    /// no allow rule for these active categories
    NoRule(Vec<String>),
    /// the check itself failed (not Windows, PowerShell missing, ...)
    Unknown(String),
}

/// One inbound rule naming the exe: action, enabled, profiles.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub allow: bool,
    pub enabled: bool,
    pub profiles: Vec<String>,
}

/// The categories a rule's `Profile` field covers ("Any", "Private, Public", ...).
fn profiles(field: &str) -> Vec<String> {
    let f = field.trim();
    if f.eq_ignore_ascii_case("any") || f.is_empty() {
        return vec!["Domain".into(), "Private".into(), "Public".into()];
    }
    f.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// `DomainAuthenticated` (Get-NetConnectionProfile) is the `Domain` profile.
fn category(c: &str) -> String {
    let c = c.trim();
    if c.starts_with("Domain") {
        "Domain".into()
    } else {
        c.to_string()
    }
}

/// Parses the check script's output: `R|<Action>|<Enabled>|<Profile>` per
/// inbound rule naming the exe, `N|<NetworkCategory>` per connected network.
pub fn parse(out: &str) -> (Vec<Rule>, Vec<String>) {
    let mut rules = Vec::new();
    let mut nets = Vec::new();
    for l in out.lines() {
        let p: Vec<&str> = l.trim().split('|').collect();
        match p.as_slice() {
            ["R", action, enabled, prof] => rules.push(Rule {
                allow: action.trim().eq_ignore_ascii_case("allow"),
                enabled: enabled.trim().eq_ignore_ascii_case("true"),
                profiles: profiles(prof),
            }),
            ["N", cat] => {
                let c = category(cat);
                if !c.is_empty() && !nets.contains(&c) {
                    nets.push(c);
                }
            }
            _ => {}
        }
    }
    (rules, nets)
}

/// Windows' rule: a matching enabled block rule wins over any allow rule.
pub fn judge(rules: &[Rule], nets: &[String]) -> Verdict {
    if nets.is_empty() {
        return Verdict::Unknown("no connected network".into());
    }
    let covers = |r: &Rule, n: &String| r.enabled && r.profiles.iter().any(|p| p.eq_ignore_ascii_case(n));
    let blocked: Vec<String> = nets.iter().filter(|n| rules.iter().any(|r| !r.allow && covers(r, n))).cloned().collect();
    if !blocked.is_empty() {
        return Verdict::Blocked(blocked);
    }
    let missing: Vec<String> = nets.iter().filter(|n| !rules.iter().any(|r| r.allow && covers(r, n))).cloned().collect();
    if !missing.is_empty() {
        return Verdict::NoRule(missing);
    }
    Verdict::Allowed(nets.to_vec())
}

/// PowerShell single-quoted literal.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The read-only check script for `exe`.
pub fn check_script(exe: &str) -> String {
    format!(
        "$e = {}; \
         Get-NetFirewallApplicationFilter -ErrorAction SilentlyContinue | Where-Object {{ $_.Program -and ([Environment]::ExpandEnvironmentVariables($_.Program) -ieq $e) }} | \
         Get-NetFirewallRule -ErrorAction SilentlyContinue | Where-Object {{ \"$($_.Direction)\" -eq 'Inbound' }} | \
         ForEach-Object {{ 'R|' + $_.Action + '|' + $_.Enabled + '|' + $_.Profile }}; \
         Get-NetConnectionProfile -ErrorAction SilentlyContinue | ForEach-Object {{ 'N|' + $_.NetworkCategory }}",
        ps_quote(exe)
    )
}

fn powershell(script: &str) -> std::io::Result<std::process::Output> {
    let mut c = Command::new("powershell");
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c.output()
}

/// Runs the check for the running exe (blocking: call it on a thread).
pub fn check() -> Verdict {
    if !cfg!(windows) {
        return Verdict::Unknown("not Windows".into());
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(e) => return Verdict::Unknown(e.to_string()),
    };
    match powershell(&check_script(&exe)) {
        Ok(o) => {
            let (rules, nets) = parse(&String::from_utf8_lossy(&o.stdout));
            judge(&rules, &nets)
        }
        Err(e) => Verdict::Unknown(e.to_string()),
    }
}

/// The `netsh` arguments that allow `exe` in on UDP for `profiles`.
pub fn allow_args(exe: &str, profiles: &[String]) -> String {
    let prof = if profiles.is_empty() { "private".to_string() } else { profiles.iter().map(|p| p.to_ascii_lowercase()).collect::<Vec<_>>().join(",") };
    format!("advfirewall firewall add rule name=\"Tremulous (Godot port)\" dir=in action=allow enable=yes protocol=udp profile={prof} program=\"{exe}\"")
}

/// The script the elevated PowerShell runs (dec054): the inbound block
/// rules naming `exe` go (Windows makes one when its own "allow access"
/// prompt is cancelled, and a block wins over any allow), then the allow
/// rule of `allow_args` is added.
pub fn fix_script(exe: &str, profiles: &[String]) -> String {
    format!(
        "$e = {}; \
         Get-NetFirewallApplicationFilter -ErrorAction SilentlyContinue | Where-Object {{ $_.Program -and ([Environment]::ExpandEnvironmentVariables($_.Program) -ieq $e) }} | \
         Get-NetFirewallRule -ErrorAction SilentlyContinue | Where-Object {{ \"$($_.Direction)\" -eq 'Inbound' -and \"$($_.Action)\" -eq 'Block' }} | Remove-NetFirewallRule; \
         netsh {}; exit $LASTEXITCODE",
        ps_quote(exe),
        allow_args(exe, profiles)
    )
}

/// `-EncodedCommand`'s argument: base64 of the UTF-16LE script (no
/// quoting through Start-Process's argument list).
pub fn encode_command(script: &str) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Asks Windows to let the game in (blocking until the player answers:
/// call it on a thread): Windows shows its administrator prompt; nothing
/// changes unless the player approves it there.
pub fn request_allow(profiles: &[String]) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("only on Windows".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
    let script = format!(
        "try {{ $p = Start-Process -FilePath powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-EncodedCommand','{}'; exit $p.ExitCode }} catch {{ exit 1223 }}",
        encode_command(&fix_script(&exe, profiles))
    );
    let o = powershell(&script).map_err(|e| e.to_string())?;
    match o.status.code() {
        Some(0) => Ok(()),
        Some(1223) => Err("the administrator prompt was declined: other PCs still cannot reach the game".into()),
        c => Err(format!("Windows did not add the rule (exit {c:?})")),
    }
}

/// The ids of the answers (what `Prompt::buttons` pairs with its labels).
pub const ANSWER_ALLOW: i64 = 0;
/// Host without the rule (the default: closing the dialog means this).
pub const ANSWER_HOST: i64 = 1;
/// Host without it and never ask when hosting (net_firewallPrompt 0).
pub const ANSWER_NEVER: i64 = 2;
/// The question at host time (dec076): Windows' TaskDialog layout, WHY as
/// the main instruction, the consequence as content, the details as footer.
#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub title: String,
    pub main: String,
    pub content: String,
    pub footer: String,
    /// (answer id, label) in display order
    pub buttons: Vec<(i64, String)>,
}

impl Prompt {
    /// The whole question as plain text, for Godot's own dialog (the fallback).
    pub fn plain(&self) -> String {
        let content = self
            .content
            .replace(&format!("<a href=\"{}\">", crate::hoststatus::HELP_URL), "")
            .replace("</a>", "");
        format!("{}\n\n{}\n\n{}", self.main, content, self.footer)
    }
}

fn lacking(v: &Verdict) -> Option<&Vec<String>> {
    match v {
        Verdict::NoRule(n) | Verdict::Blocked(n) => Some(n),
        _ => None,
    }
}

/// Whether the host-time question is asked (dec076): only when the host
/// starts a network game (`host_time`, not the startup check), the firewall
/// lacks a rule, the player did not say never (net_firewallPrompt 0, which
/// also keeps what the startup prompt's old "Don't ask again" saved), it was
/// not asked already this run, and no administrator prompt is pending.
pub fn should_ask(v: &Verdict, setting: i64, asked: bool, fixing: bool, host_time: bool) -> bool {
    host_time && setting != 0 && !asked && !fixing && lacking(v).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_rule_on_private_network() {
        let (r, n) = parse("N|Private\r\n");
        assert_eq!(judge(&r, &n), Verdict::NoRule(vec!["Private".into()]));
    }

    #[test]
    fn allow_rule_for_private_covers_private_only() {
        let out = "R|Allow|True|Private\nN|Private\n";
        let (r, n) = parse(out);
        assert_eq!(judge(&r, &n), Verdict::Allowed(vec!["Private".into()]));
        let (r, n) = parse("R|Allow|True|Private\nN|Public\n");
        assert_eq!(judge(&r, &n), Verdict::NoRule(vec!["Public".into()]));
    }

    #[test]
    fn block_wins_and_disabled_rules_do_not_count() {
        let (r, n) = parse("R|Allow|True|Any\nR|Block|True|Public\nN|Public\n");
        assert_eq!(judge(&r, &n), Verdict::Blocked(vec!["Public".into()]));
        let (r, n) = parse("R|Allow|False|Private\nN|Private\n");
        assert_eq!(judge(&r, &n), Verdict::NoRule(vec!["Private".into()]));
        let (r, n) = parse("R|Allow|True|Any\nR|Block|False|Private\nN|DomainAuthenticated\nN|Private\n");
        assert_eq!(judge(&r, &n), Verdict::Allowed(vec!["Domain".into(), "Private".into()]));
    }

    #[test]
    fn multi_profile_field_and_unknown() {
        let (r, n) = parse("R|Allow|True|Private, Public\nN|Public\n");
        assert_eq!(judge(&r, &n), Verdict::Allowed(vec!["Public".into()]));
        assert!(matches!(judge(&[], &[]), Verdict::Unknown(_)));
    }

    #[test]
    fn scripts_quote_paths() {
        let s = check_script(r"C:\Users\O'Neil\Desktop\Tremulous Release\Tremulous.exe");
        assert!(s.starts_with(r"$e = 'C:\Users\O''Neil\Desktop\Tremulous Release\Tremulous.exe';"));
        let a = allow_args(r"C:\x y\T.exe", &["Private".into(), "Public".into()]);
        assert!(a.ends_with("profile=private,public program=\"C:\\x y\\T.exe\""));
        assert!(a.contains("dir=in action=allow") && a.contains("protocol=udp"));
        assert!(allow_args("a", &[]).contains("profile=private "));
        let f = fix_script(r"C:\O'Neil\T.exe", &["Public".into()]);
        assert!(f.starts_with(r"$e = 'C:\O''Neil\T.exe';") && f.contains("-eq 'Block' } | Remove-NetFirewallRule;"));
        assert!(f.contains("netsh advfirewall firewall add rule") && f.contains("profile=public"));
    }

    #[test]
    fn encoded_command_is_utf16le_base64() {
        // PowerShell: [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes('dir'))
        assert_eq!(encode_command("dir"), "ZABpAHIA");
        assert_eq!(encode_command("ab"), "YQBiAA==");
        assert_eq!(encode_command(""), "");
    }

    #[test]
    fn host_time_decision() {
        let lack = Verdict::NoRule(vec!["Private".into()]);
        let blocked = Verdict::Blocked(vec!["Public".into()]);
        // asks at host time, when lacking a rule
        assert!(should_ask(&lack, 1, false, false, true));
        assert!(should_ask(&blocked, 1, false, false, true));
        // never at startup (the silent check)
        assert!(!should_ask(&lack, 1, false, false, false));
        // "Don't ask when I host" (and the old startup opt-out: net_firewallPrompt 0)
        assert!(!should_ask(&lack, 0, false, false, true));
        // once per run, and not while the administrator prompt is pending
        assert!(!should_ask(&lack, 1, true, false, true));
        assert!(!should_ask(&lack, 1, false, true, true));
        // nothing to ask when allowed or unknown
        assert!(!should_ask(
            &Verdict::Allowed(vec!["Private".into()]),
            1,
            false,
            false,
            true
        ));
        assert!(!should_ask(
            &Verdict::Unknown("x".into()),
            1,
            false,
            false,
            true
        ));
    }
}
