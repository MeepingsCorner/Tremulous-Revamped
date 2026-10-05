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

/// Asks Windows to add the allow rule: Windows shows its administrator
/// prompt; nothing changes unless the player approves it there.
pub fn request_allow(profiles: &[String]) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("only on Windows".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
    let script = format!("Start-Process -FilePath netsh -Verb RunAs -WindowStyle Hidden -ArgumentList {}", ps_quote(&allow_args(&exe, profiles)));
    let o = powershell(&script).map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(())
    } else {
        Err("Windows did not add the rule (the administrator prompt was declined?)".into())
    }
}

/// The host's line for a verdict on `port`.
pub fn message(v: &Verdict, port: u16) -> String {
    match v {
        Verdict::Allowed(n) => format!("Windows Firewall: other PCs can reach this game ({} network)", n.join(", ")),
        Verdict::Blocked(n) => format!("Windows Firewall BLOCKS this game on the {} network: other PCs cannot join or see it. Remove the block rule for this program in Windows Security > Firewall > Allow an app, or type firewall_allow (after removing it)", n.join(", ")),
        Verdict::NoRule(n) => format!("Windows Firewall has no rule letting other PCs reach this game on the {} network, so they cannot join or see it (UDP {port}). Type firewall_allow in the console (Windows asks for administrator approval), or tick it in Windows Security > Firewall > Allow an app", n.join(", ")),
        Verdict::Unknown(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_rule_on_private_network() {
        let (r, n) = parse("N|Private\r\n");
        assert_eq!(judge(&r, &n), Verdict::NoRule(vec!["Private".into()]));
        assert!(message(&judge(&r, &n), 30720).contains("firewall_allow"));
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
        assert_eq!(message(&Verdict::Unknown("x".into()), 1), "");
    }

    #[test]
    fn scripts_quote_paths() {
        let s = check_script(r"C:\Users\O'Neil\Desktop\Tremulous Release\Tremulous.exe");
        assert!(s.starts_with(r"$e = 'C:\Users\O''Neil\Desktop\Tremulous Release\Tremulous.exe';"));
        let a = allow_args(r"C:\x y\T.exe", &["Private".into(), "Public".into()]);
        assert!(a.ends_with("profile=private,public program=\"C:\\x y\\T.exe\""));
        assert!(a.contains("dir=in action=allow") && a.contains("protocol=udp"));
        assert!(allow_args("a", &[]).contains("profile=private "));
    }
}
