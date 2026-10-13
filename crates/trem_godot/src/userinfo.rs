//! The client's userinfo for the local player: the `name` cvar
//! (cl_main.c `CL_Init`; the port's default is not the OS user, dec064), `CL_CheckUserinfo` (a changed
//! CVAR_USERINFO cvar sends a new userinfo; the listen server runs the
//! game's `ClientUserinfoChanged`, trem_sim `client::userinfo_changed`,
//! with its rename rules: g_minNameChangePeriod, g_maxNameChanges, mute,
//! the admin name check, the censor, the namelog and the ClientRename log
//! line) and cg_players.c `CG_NewClientInfo` / `CG_StatusMessages`: the
//! "renamed to", "joined the", "left the" prints made when a client's name
//! or team in its CS_PLAYERS info changes. Pure Rust.

use crate::cmd::{Cvars, CVAR_ARCHIVE, CVAR_USERINFO};

/// The 1.1 client took the default from `Sys_GetCurrentUser` (the OS login
/// name); the port does not (dec064): the default is "UnnamedPlayer" until
/// `randname::assign_if_unchosen` gives a profile without a chosen name a
/// random one, after autogen.cfg has run.
/// `CL_Init`'s userinfo cvars that the game reads (`name`; `rate` is the ui's).
pub fn register_cvars(c: &mut Cvars) {
    c.get("name", "UnnamedPlayer", CVAR_USERINFO | CVAR_ARCHIVE);
}

/// `CL_CheckUserinfo`: true (and the flag taken) when a CVAR_USERINFO cvar
/// changed since the last userinfo was sent.
pub fn take_userinfo_modified(c: &mut Cvars) -> bool {
    let m = c.modified_flags & CVAR_USERINFO != 0;
    c.modified_flags &= !CVAR_USERINFO;
    m
}

/// The cgame's `clientInfo_t`s (`ci->infoValid`, `ci->name`, `ci->team`).
#[derive(Default, Debug, Clone)]
pub struct ClientNames {
    names: Vec<Option<(String, i32)>>,
}

impl ClientNames {
    /// Forget every client (`CG_Init`: a new map's gamestate).
    pub fn clear(&mut self) {
        self.names.clear();
    }

    /// `CG_NewClientInfo` for each client slot of the new snapshot
    /// (`connected`, name, team: 0 none, 1 aliens, 2 humans):
    /// `CG_StatusMessages` for every client whose info was valid. An empty
    /// slot clears its info (`CG_ConfigStringModified` with an empty
    /// CS_PLAYERS string).
    pub fn update<'a>(&mut self, clients: impl Iterator<Item = (bool, &'a str, i32)>) -> Vec<String> {
        let mut out = Vec::new();
        for (i, (connected, name, team)) in clients.enumerate() {
            if self.names.len() <= i {
                self.names.resize(i + 1, None);
            }
            if !connected {
                self.names[i] = None;
                continue;
            }
            if let Some((old, old_team)) = &self.names[i] {
                out.extend(status_messages(old, *old_team, name, team));
            }
            self.names[i] = Some((name.to_string(), team));
        }
        out
    }
}

/// `CG_StatusMessages`: print messages for player status changes.
pub fn status_messages(old: &str, old_team: i32, new: &str, new_team: i32) -> Vec<String> {
    let mut out = Vec::new();
    if old != new {
        out.push(format!("{old}^7 renamed to {new}\n"));
    }
    if old_team != new_team {
        out.push(if new_team == 0 {
            format!("{new}^7 left the {}s\n", trem_sim::kernel::bg::team_name(old_team))
        } else if old_team == 0 {
            format!("{new}^7 joined the {}s\n", trem_sim::kernel::bg::team_name(new_team))
        } else {
            format!("{new}^7 left the {}s and joined the {}s\n", trem_sim::kernel::bg::team_name(old_team), trem_sim::kernel::bg::team_name(new_team))
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_user_default_and_userinfo_flag() {
        let mut c = Cvars::default();
        register_cvars(&mut c);
        assert!(!c.string("name").is_empty());
        assert!(take_userinfo_modified(&mut c));
        assert!(!take_userinfo_modified(&mut c));
        c.set("name", "Granger");
        assert!(take_userinfo_modified(&mut c));
        // a non-userinfo cvar does not send userinfo
        c.get("cg_foo", "0", CVAR_ARCHIVE);
        c.set("cg_foo", "1");
        assert!(!take_userinfo_modified(&mut c));
    }

    #[test]
    fn status_messages_only_for_valid_infos() {
        let mut n = ClientNames::default();
        assert!(n.update([(true, "A", 0), (false, "", 0)].into_iter()).is_empty());
        assert_eq!(n.update([(true, "B", 0), (true, "C", 0)].into_iter()), vec!["A^7 renamed to B\n".to_string()]);
        // a slot that emptied and was reused is a new client, not a rename
        assert!(n.update([(true, "B", 0), (false, "", 0)].into_iter()).is_empty());
        assert!(n.update([(true, "B", 0), (true, "D", 0)].into_iter()).is_empty());
        // team changes
        assert_eq!(n.update([(true, "B", 2), (true, "D", 0)].into_iter()), vec!["B^7 joined the humans\n".to_string()]);
        assert_eq!(n.update([(true, "B", 1), (true, "D", 0)].into_iter()), vec!["B^7 left the humans and joined the aliens\n".to_string()]);
        assert_eq!(n.update([(true, "E", 0), (true, "D", 0)].into_iter()), vec!["B^7 renamed to E\n".to_string(), "E^7 left the aliens\n".to_string()]);
    }
}
