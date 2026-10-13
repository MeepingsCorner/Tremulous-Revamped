//! The host-time status of a hosted game (dec076): can LAN
//! friends and internet friends join, and if not, which step is missing.
//!
//! Step 1 is this PC's Windows Firewall (needed by everyone), step 2 is the
//! router's port forwarding (UPnP; needed only by friends on the internet).
//! The dialog and the console print the same facts in the same order. Plain
//! words only: the Windows network profile names never reach the player.

use crate::firewall::{Prompt, Verdict, ANSWER_ALLOW, ANSWER_HOST, ANSWER_NEVER};
use crate::upnp::Mapping;
use std::net::Ipv4Addr;

/// The fixed help page for forwarding a port (the only link the dialog opens).
pub const HELP_URL: &str = "https://portforward.com";
/// How long the dialog waits for the router's answer before it opens.
pub const UPNP_WAIT_SECS: f64 = 4.0;

/// Step 1, as the player sees it.
#[derive(Clone, Debug, PartialEq)]
pub enum Fw {
    /// not Windows: there is no step 1
    NotApplicable,
    Allowed,
    /// no rule lets the game in
    NoRule,
    /// a rule blocks the game
    BlockRule,
    /// the check itself failed
    Unchecked,
}

/// Step 2, as the player sees it.
#[derive(Clone, Debug, PartialEq)]
pub enum Router {
    /// net_upnp 0: the player turned it off
    Off,
    NoAnswer,
    /// the router said no, with its UPnP error code
    Refused(String),
    Failed(String),
    Forwarded {
        ext: Option<String>,
        ext_ok: bool,
    },
}

impl Fw {
    pub fn from_verdict(v: &Verdict) -> Fw {
        match v {
            Verdict::Allowed(_) => Fw::Allowed,
            Verdict::NoRule(_) => Fw::NoRule,
            Verdict::Blocked(_) => Fw::BlockRule,
            Verdict::Unknown(_) => Fw::Unchecked,
        }
    }
    fn applicable(&self) -> bool {
        *self != Fw::NotApplicable
    }
    pub fn lacks(&self) -> bool {
        matches!(self, Fw::NoRule | Fw::BlockRule)
    }
}

impl Router {
    /// From what the UPnP thread returned (`upnp::map_udp`).
    pub fn from_result(r: &Result<Mapping, String>) -> Router {
        match r {
            Ok(m) => {
                let ext = m.external_ip.clone();
                let ext_ok = ext.as_deref().is_none_or(|ip| !crate::upnp::is_private(ip));
                Router::Forwarded { ext, ext_ok }
            }
            Err(e) if e.contains("no UPnP router answered") => Router::NoAnswer,
            Err(e) => match e.split_once("UPnP error ") {
                Some((_, rest)) => {
                    let code: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if code.is_empty() {
                        Router::Failed(e.clone())
                    } else {
                        Router::Refused(code)
                    }
                }
                None => Router::Failed(e.clone()),
            },
        }
    }
    fn ok(&self) -> bool {
        matches!(self, Router::Forwarded { ext_ok: true, .. })
    }
}

/// Everything the status needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub map: String,
    pub port: u16,
    pub fw: Fw,
    pub router: Router,
    /// this PC's addresses, the one facing the default route flagged as main
    pub addrs: Vec<(Ipv4Addr, bool)>,
}

/// The addresses friends on the LAN join at: the main one first.
pub fn order_addrs(all: &[Ipv4Addr], main: Option<Ipv4Addr>) -> Vec<(Ipv4Addr, bool)> {
    let mut out: Vec<(Ipv4Addr, bool)> = Vec::new();
    for a in all {
        if a.is_loopback() || a.is_link_local() || out.iter().any(|o| o.0 == *a) {
            continue;
        }
        out.push((*a, Some(*a) == main));
    }
    // no default route known: a lone address is the main one
    if main.is_none() && out.len() == 1 {
        out[0].1 = true;
    }
    out.sort_by_key(|o| !o.1);
    out
}

/// Whether the report may be shown: the firewall answered, and the router
/// answered, or UPnP is off, or the wait is over.
pub fn ready(fw_done: bool, router_done: bool, upnp_on: bool, waited_secs: f64) -> bool {
    fw_done && (router_done || !upnp_on || waited_secs >= UPNP_WAIT_SECS)
}

impl Status {
    fn main_addr(&self) -> Option<String> {
        self.addrs
            .iter()
            .find(|a| a.1)
            .or(self.addrs.first())
            .map(|a| a.0.to_string())
    }

    fn router_step(&self) -> u8 {
        if self.fw.applicable() {
            2
        } else {
            1
        }
    }

    fn fw_text(&self) -> &'static str {
        match self.fw {
            Fw::Allowed => "allows friends in",
            Fw::NoRule => "blocks friends. No rule lets this game in",
            Fw::BlockRule => "blocks friends. A rule blocks this game",
            Fw::Unchecked => "could not be checked",
            Fw::NotApplicable => "",
        }
    }

    fn forward_hint(&self) -> String {
        match self.main_addr() {
            Some(a) => format!("Forward UDP {} to this PC ({a})", self.port),
            None => format!("Forward UDP {} to this PC", self.port),
        }
    }

    fn router_text(&self) -> String {
        let p = self.port;
        match &self.router {
            Router::Forwarded { ext: Some(ip), ext_ok: true } => {
                format!("forwarded. Internet friends join at {ip}:{p}")
            }
            Router::Forwarded { ext: Some(ip), ext_ok: false } => format!(
                "forwarded, but its internet address {ip} cannot be reached from the internet (another router or your provider sits in front). {}",
                self.forward_hint()
            ),
            Router::Forwarded { ext: None, .. } => {
                format!("forwarded. Internet friends join at your internet address, port {p}")
            }
            Router::Refused(c) if c == "718" => format!(
                "not forwarded. UPnP refused, error 718 (that port is already forwarded to another device). {}",
                self.forward_hint()
            ),
            Router::Refused(c) => {
                format!("not forwarded. UPnP refused, error {c}. {}", self.forward_hint())
            }
            Router::Failed(e) => format!("not forwarded. UPnP failed ({e}). {}", self.forward_hint()),
            Router::NoAnswer => format!("no UPnP answer. {}", self.forward_hint()),
            Router::Off => format!("not asked (net_upnp 0). {}", self.forward_hint()),
        }
    }

    /// The help link belongs where the router step is not fine and the
    /// player did not turn UPnP off on purpose.
    fn wants_link(&self) -> bool {
        !self.router.ok() && self.router != Router::Off
    }

    pub fn headline(&self) -> String {
        if self.fw == Fw::Unchecked && self.router.ok() {
            return format!("Hosting {}", self.map);
        }
        if self.fw.lacks() {
            "Friends can't join your game yet".into()
        } else if !self.router.ok() {
            "Internet friends can't join your game yet".into()
        } else {
            "Friends can join your game".into()
        }
    }

    pub fn lan_row(&self) -> String {
        match self.fw {
            Fw::NoRule | Fw::BlockRule => "LAN friends: blocked (needs step 1)".into(),
            Fw::Unchecked => "LAN friends: not sure (step 1 could not be checked)".into(),
            _ => "LAN friends: can join".into(),
        }
    }

    pub fn lan_detail(&self) -> Option<String> {
        self.fw
            .applicable()
            .then(|| format!("1. This PC's firewall: {}.", self.fw_text()))
    }

    /// "2. Your router: ..." on its own (a late answer).
    pub fn router_line(&self) -> String {
        format!(
            "{}. Your router: {}.",
            self.router_step(),
            self.router_text()
        )
    }

    pub fn internet_row(&self) -> String {
        let (f, r) = (self.fw.lacks(), !self.router.ok());
        let rs = self.router_step();
        match (f, r) {
            (true, true) => format!("Internet friends: blocked (needs steps 1 and {rs})"),
            (true, false) => "Internet friends: blocked (needs step 1)".into(),
            (false, true) => format!("Internet friends: blocked (needs step {rs})"),
            (false, false) if self.fw == Fw::Unchecked => {
                "Internet friends: not sure (step 1 could not be checked)".into()
            }
            (false, false) => "Internet friends: can join".into(),
        }
    }

    pub fn internet_detail(&self, link: &str) -> String {
        let mut s = String::new();
        if self.fw.applicable() {
            s.push_str("1. This PC's firewall: as above. ");
        }
        s.push_str(&format!(
            "{}. Your router: {}.",
            self.router_step(),
            self.router_text()
        ));
        if self.fw.lacks() && !self.router.ok() {
            s.push_str(&format!(
                " Step {} does nothing until step 1 is fixed.",
                self.router_step()
            ));
        }
        if self.wants_link() {
            s.push_str(&format!("\nHow to forward a port on your router: {link}"));
        }
        s
    }

    /// The console block (also the engine log): the same facts as the dialog.
    pub fn console(&self) -> Vec<String> {
        let mut l = vec![format!(
            "Hosting {} on UDP {}. {}",
            self.map,
            self.port,
            self.headline()
        )];
        let joins: Vec<String> = self
            .addrs
            .iter()
            .map(|(a, m)| {
                format!(
                    "{a}:{}{}",
                    self.port,
                    if *m { " (main)" } else { " (other adapter)" }
                )
            })
            .collect();
        let mut lan = self.lan_row();
        if !joins.is_empty() {
            lan.push_str(&format!(". Join at {}", joins.join(", ")));
        }
        l.push(lan);
        if let Some(d) = self.lan_detail() {
            l.push(format!("  {d}"));
        }
        l.push(self.internet_row());
        for d in self.internet_detail(HELP_URL).lines() {
            l.push(format!("  {d}"));
        }
        if self.fw.lacks() {
            l.push("Next: type firewall_allow in the console (Windows asks for administrator approval).".into());
        }
        l
    }

    /// The dialog: only when the player can fix something (step 1).
    pub fn prompt(&self, exe: &str) -> Option<Prompt> {
        if !self.fw.lacks() {
            return None;
        }
        let link = format!("<a href=\"{HELP_URL}\">portforward.com</a>");
        let mut content = format!("{}\n", self.lan_row());
        content.push_str(&format!("   {}\n\n", self.lan_detail().unwrap_or_default()));
        content.push_str(&format!("{}\n", self.internet_row()));
        content.push_str(&format!(
            "   {}",
            self.internet_detail(&link).replace('\n', "\n   ")
        ));
        let what = if self.fw == Fw::BlockRule {
            "Allow removes the blocking rule for this program and adds one Windows Firewall rule"
        } else {
            "Allow adds one Windows Firewall rule"
        };
        let footer = format!(
            "{what}: incoming UDP for {exe}, on the network this PC is on now, used only while you host. The rule belongs to this exact file path: a copy of the game in another folder asks again. Risk: small. While hosting, anyone who finds your server can send it data, so a bug in the game could be abused. Windows asks for administrator permission (UAC, it says \"Windows PowerShell\"), then PowerShell adds the rule in the background."
        );
        Some(Prompt {
            title: format!("Tremulous: hosting {}", self.map),
            main: self.headline(),
            content,
            footer,
            buttons: vec![
                (ANSWER_ALLOW, "Allow friends in".into()),
                (ANSWER_HOST, "Host without it".into()),
                (ANSWER_NEVER, "Don't ask when I host".into()),
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(fw: Fw, router: Router) -> Status {
        Status {
            map: "atcs".into(),
            port: 30720,
            fw,
            router,
            addrs: order_addrs(
                &[
                    "10.20.30.4".parse().unwrap(),
                    "10.0.0.5".parse().unwrap(),
                    "127.0.0.1".parse().unwrap(),
                ],
                Some("10.0.0.5".parse().unwrap()),
            ),
        }
    }
    fn fwd() -> Router {
        Router::Forwarded {
            ext: Some("203.0.113.7".into()),
            ext_ok: true,
        }
    }
    fn all_text(s: &Status) -> String {
        let mut t = s.console().join("\n");
        if let Some(p) = s.prompt(r"C:\Games\Tremulous-Revamped\Tremulous.exe") {
            t.push_str(&format!(
                "\n{}\n{}\n{}\n{}",
                p.title, p.main, p.content, p.footer
            ));
            for b in &p.buttons {
                t.push_str(&format!("\n{}", b.1));
            }
            t.push_str(&p.plain());
        }
        t
    }

    #[test]
    fn blocked_both_steps_718() {
        let s = st(Fw::NoRule, Router::Refused("718".into()));
        let c = s.console();
        assert_eq!(
            c[0],
            "Hosting atcs on UDP 30720. Friends can't join your game yet"
        );
        assert_eq!(
            c[1],
            "LAN friends: blocked (needs step 1). Join at 10.0.0.5:30720 (main), 10.20.30.4:30720 (other adapter)"
        );
        assert_eq!(
            c[2],
            "  1. This PC's firewall: blocks friends. No rule lets this game in."
        );
        assert_eq!(c[3], "Internet friends: blocked (needs steps 1 and 2)");
        assert!(c[4].contains("1. This PC's firewall: as above. 2. Your router: not forwarded."));
        assert!(c[4].contains(
            "UPnP refused, error 718 (that port is already forwarded to another device)."
        ));
        assert!(c[4].contains("Forward UDP 30720 to this PC (10.0.0.5)."));
        assert!(c[4].contains("Step 2 does nothing until step 1 is fixed."));
        assert_eq!(
            c[5],
            "  How to forward a port on your router: https://portforward.com"
        );
        assert!(c[6].starts_with("Next: type firewall_allow"));
    }

    #[test]
    fn every_outcome() {
        // all fine
        let s = st(Fw::Allowed, fwd());
        assert_eq!(s.headline(), "Friends can join your game");
        assert_eq!(s.lan_row(), "LAN friends: can join");
        assert_eq!(s.internet_row(), "Internet friends: can join");
        assert!(s
            .internet_detail("L")
            .contains("2. Your router: forwarded. Internet friends join at 203.0.113.7:30720."));
        assert!(!s
            .console()
            .iter()
            .any(|l| l.contains("Next:") || l.contains("portforward")));
        assert!(s.prompt("x").is_none());
        // block rule
        let s = st(Fw::BlockRule, fwd());
        assert_eq!(
            s.lan_detail().unwrap(),
            "1. This PC's firewall: blocks friends. A rule blocks this game."
        );
        assert_eq!(s.internet_row(), "Internet friends: blocked (needs step 1)");
        let p = s.prompt("x.exe").unwrap();
        assert!(p.footer.starts_with("Allow removes the blocking rule"));
        assert!(!p.content.contains("Step 2 does nothing"));
        // firewall fine, router trouble: console only, no dialog
        let s = st(Fw::Allowed, Router::NoAnswer);
        assert_eq!(s.headline(), "Internet friends can't join your game yet");
        assert_eq!(s.lan_row(), "LAN friends: can join");
        assert_eq!(s.internet_row(), "Internet friends: blocked (needs step 2)");
        assert!(s.internet_detail("L").contains(
            "2. Your router: no UPnP answer. Forward UDP 30720 to this PC (10.0.0.5)."
        ));
        assert!(s.prompt("x").is_none());
        // router behind another router
        let s = st(
            Fw::Allowed,
            Router::Forwarded {
                ext: Some("10.1.2.3".into()),
                ext_ok: false,
            },
        );
        assert!(s.internet_detail("L").contains("forwarded, but its internet address 10.1.2.3 cannot be reached from the internet (another router or your provider sits in front)"));
        // UPnP off on purpose: the plain hint, no link
        let s = st(Fw::Allowed, Router::Off);
        assert!(s
            .internet_detail("L")
            .contains("not asked (net_upnp 0). Forward UDP 30720 to this PC (10.0.0.5)"));
        assert!(!s.internet_detail("L").contains("How to forward"));
        assert!(!s.console().join("\n").contains("portforward"));
        // firewall check failed
        let s = st(Fw::Unchecked, fwd());
        assert_eq!(s.headline(), "Hosting atcs");
        assert_eq!(
            s.lan_detail().unwrap(),
            "1. This PC's firewall: could not be checked."
        );
        assert_eq!(
            s.lan_row(),
            "LAN friends: not sure (step 1 could not be checked)"
        );
        assert!(s.prompt("x").is_none());
        // not Windows: only the router step, numbered 1
        let s = st(Fw::NotApplicable, Router::Refused("501".into()));
        assert_eq!(s.lan_detail(), None);
        assert_eq!(s.internet_row(), "Internet friends: blocked (needs step 1)");
        assert!(s
            .internet_detail("L")
            .starts_with("1. Your router: not forwarded. UPnP refused, error 501."));
        assert!(!s.internet_detail("L").contains("does nothing"));
    }

    #[test]
    fn help_link_only_when_the_router_step_is_not_fine() {
        let bad = st(Fw::NoRule, Router::Refused("718".into()));
        let p = bad.prompt(r"C:\T\Tremulous.exe").unwrap();
        assert!(p.content.contains("How to forward a port on your router: <a href=\"https://portforward.com\">portforward.com</a>"));
        assert!(p
            .plain()
            .contains("How to forward a port on your router: portforward.com"));
        assert!(!p.plain().contains("<a"));
        let fine = st(Fw::NoRule, fwd());
        assert!(!fine.prompt("x").unwrap().content.contains("portforward"));
        let off = st(Fw::NoRule, Router::Off);
        assert!(!off.prompt("x").unwrap().content.contains("portforward"));
        assert!(off
            .prompt("x")
            .unwrap()
            .content
            .contains("Forward UDP 30720 to this PC (10.0.0.5)"));
    }

    #[test]
    fn dialog_shape() {
        let s = st(Fw::NoRule, Router::Refused("718".into()));
        let exe = r"C:\Games\Tremulous-Revamped\Tremulous.exe";
        let p = s.prompt(exe).unwrap();
        assert_eq!(p.title, "Tremulous: hosting atcs");
        assert_eq!(p.main, "Friends can't join your game yet");
        let labels: Vec<&str> = p.buttons.iter().map(|b| b.1.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Allow friends in",
                "Host without it",
                "Don't ask when I host"
            ]
        );
        let ids: Vec<i64> = p.buttons.iter().map(|b| b.0).collect();
        assert_eq!(ids, [ANSWER_ALLOW, ANSWER_HOST, ANSWER_NEVER]);
        assert!(p.content.contains(
            "LAN friends: blocked (needs step 1)\n   1. This PC's firewall: blocks friends."
        ));
        assert!(p
            .content
            .contains("Internet friends: blocked (needs steps 1 and 2)"));
        for k in [
            exe,
            "incoming UDP",
            "only while you host",
            "Risk: small",
            "administrator",
            "UAC",
            "PowerShell",
        ] {
            assert!(p.footer.contains(k), "{k}");
        }
    }

    #[test]
    fn no_profile_names_in_player_visible_text() {
        let fws = [
            Fw::NotApplicable,
            Fw::Allowed,
            Fw::NoRule,
            Fw::BlockRule,
            Fw::Unchecked,
        ];
        let routers = [
            Router::Off,
            Router::NoAnswer,
            Router::Refused("718".into()),
            Router::Refused("501".into()),
            Router::Failed("x".into()),
            fwd(),
            Router::Forwarded {
                ext: Some("10.0.0.1".into()),
                ext_ok: false,
            },
            Router::Forwarded {
                ext: None,
                ext_ok: true,
            },
        ];
        for f in &fws {
            for r in &routers {
                let t = all_text(&st(f.clone(), r.clone())).to_ascii_lowercase();
                for w in ["public", "private", "domain"] {
                    assert!(!t.contains(w), "{w} in {f:?} {r:?}: {t}");
                }
            }
        }
    }

    #[test]
    fn router_from_upnp_results() {
        let e = |s: &str| Router::from_result(&Err(s.to_string()));
        assert_eq!(
            e("no UPnP router answered (UPnP off, or not a home router)"),
            Router::NoAnswer
        );
        assert_eq!(
            e("the router refused the port mapping (UPnP error 718 ConflictInMappingEntry)"),
            Router::Refused("718".into())
        );
        assert!(matches!(
            e("the router's description answered HTTP 404"),
            Router::Failed(_)
        ));
        let m = Mapping {
            control_url: String::new(),
            service: String::new(),
            port: 30720,
            local_ip: "10.0.0.5".parse().unwrap(),
            external_ip: Some("100.72.0.5".into()),
        };
        assert_eq!(
            Router::from_result(&Ok(m.clone())),
            Router::Forwarded {
                ext: Some("100.72.0.5".into()),
                ext_ok: false
            }
        );
        let m = Mapping {
            external_ip: Some("203.0.113.7".into()),
            ..m
        };
        assert_eq!(Router::from_result(&Ok(m)), fwd());
    }

    #[test]
    fn forward_hint_without_an_address() {
        let mut s = st(Fw::Allowed, Router::NoAnswer);
        s.addrs.clear();
        assert_eq!(
            s.router_line(),
            "2. Your router: no UPnP answer. Forward UDP 30720 to this PC."
        );
        assert_eq!(s.console()[1], "LAN friends: can join");
    }

    #[test]
    fn addresses_main_first_and_filtered() {
        let a = order_addrs(
            &[
                "10.20.30.4".parse().unwrap(),
                "127.0.0.1".parse().unwrap(),
                "169.254.1.1".parse().unwrap(),
                "10.0.0.5".parse().unwrap(),
                "10.0.0.5".parse().unwrap(),
            ],
            Some("10.0.0.5".parse().unwrap()),
        );
        assert_eq!(
            a,
            vec![
                ("10.0.0.5".parse().unwrap(), true),
                ("10.20.30.4".parse().unwrap(), false)
            ]
        );
        // no default route: a lone address is main, several are all "other adapter"
        assert_eq!(
            order_addrs(&["10.0.0.5".parse().unwrap()], None),
            vec![("10.0.0.5".parse().unwrap(), true)]
        );
        assert!(order_addrs(
            &["10.0.0.5".parse().unwrap(), "10.0.0.6".parse().unwrap()],
            None
        )
        .iter()
        .all(|x| !x.1));
    }

    #[test]
    fn the_upnp_wait_is_four_seconds() {
        // firewall not answered: never ready
        assert!(!ready(false, true, true, 10.0));
        // router answered, or UPnP off: ready at once
        assert!(ready(true, true, true, 0.0));
        assert!(ready(true, false, false, 0.0));
        // router silent: waits, then gives up at 4 s
        assert!(!ready(true, false, true, 0.0));
        assert!(!ready(true, false, true, 3.9));
        assert!(ready(true, false, true, 4.0));
    }
}
