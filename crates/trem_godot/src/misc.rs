//! Client-side state the port's TremGame keeps for the smaller cgame /
//! client pieces (one field in game.rs): the cgame's client names
//! (userinfo.rs), the vote talk beep (cg_draw.c `CG_DrawVote`) and the chat
//! talk sounds (the sound half of cg_servercmds.c CG_Say), the
//! gamepad (joystick.rs), the console's view of the printed text
//! (console.rs) and the cgame debug commands (devcmds.rs).

use crate::userinfo::ClientNames;

/// `cgs.voteTime` / `voteYes` / `voteNo` per team as last seen, and
/// `cgs.voteModified` (cg_servercmds.c `CG_ConfigStringModified`: any of
/// CS_VOTE_TIME / CS_VOTE_YES / CS_VOTE_NO changing sets it).
#[derive(Default, Debug, Clone)]
pub struct VoteBeep {
    seen: [(i32, i32, i32); 3],
    modified: [bool; 3],
    started: bool,
}

impl VoteBeep {
    /// The new snapshot's vote configstrings ((time, yes, no) per team);
    /// the first call is the gamestate (`CG_SetConfigValues` does not mark
    /// votes modified).
    pub fn update(&mut self, votes: [(i32, i32, i32); 3]) {
        for t in 0..3 {
            if self.started && votes[t] != self.seen[t] {
                self.modified[t] = true;
            }
        }
        self.seen = votes;
        self.started = true;
    }

    /// `CG_DrawVote( team )`'s beep: true when a talk sound plays ("play a
    /// talk beep whenever it is modified", only while the vote is up).
    pub fn draw(&mut self, team: usize) -> bool {
        if self.seen[team].0 == 0 {
            return false;
        }
        std::mem::take(&mut self.modified[team])
    }

    /// A new map (`CG_Init`).
    pub fn reset(&mut self) {
        *self = VoteBeep::default();
    }
}

/// See the module doc.
#[derive(Default)]
pub struct Misc {
    pub names: ClientNames,
    pub vote: VoteBeep,
    pub joy: crate::joystick::JoyState,
    /// feed lines (dropped + len) already copied into the console
    pub feed_seen: u64,
    pub dev: crate::devcmds::DevState,
    pub con: crate::console::Console,
    /// `CL_LoadConsoleHistory` ran (on the first use of the console)
    pub con_history_loaded: bool,
    /// chat lines this frame (the talk sounds of CG_Say)
    pub chat_sounds: Vec<trem_sim::kernel::cmds::SayMode>,
}

/// A snapshot of what tab completion enumerates (`Cmd_CommandCompletion`,
/// `Cvar_CommandCompletion`, `FS_FilenameCompletion` of the game folder).
pub struct Completions {
    pub commands: Vec<String>,
    pub cvars: Vec<(String, String)>,
    /// cfg / txt files of the user folder and the pk3s' root
    pub files: Vec<String>,
}

impl crate::console::CompletionSource for Completions {
    fn commands(&self) -> Vec<String> {
        self.commands.clone()
    }
    fn cvars(&self) -> Vec<(String, String)> {
        self.cvars.clone()
    }
    fn files(&self, dir: &str, ext: &str, strip: bool) -> Vec<String> {
        let suffix = format!(".{}", ext.to_ascii_lowercase());
        self.files
            .iter()
            .filter(|f| dir.is_empty() && f.to_ascii_lowercase().ends_with(&suffix))
            .map(|f| if strip { f[..f.len() - suffix.len()].to_string() } else { f.clone() })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vote_beeps_on_call_and_on_each_change() {
        let mut v = VoteBeep::default();
        v.update([(0, 0, 0); 3]);
        assert!(!v.draw(0));
        // callvote: CS_VOTE_TIME and CS_VOTE_YES change
        v.update([(5000, 1, 0), (0, 0, 0), (0, 0, 0)]);
        assert!(v.draw(0));
        assert!(!v.draw(0));
        // a vote no
        v.update([(5000, 1, 1), (0, 0, 0), (0, 0, 0)]);
        assert!(v.draw(0));
        // a team vote is only drawn (and beeps) for that team
        v.update([(5000, 1, 1), (7000, 1, 0), (0, 0, 0)]);
        assert!(!v.draw(2));
        assert!(v.draw(1));
        // the vote ends: CS_VOTE_TIME "" -> no beep while it is not drawn
        v.update([(0, 0, 0), (7000, 1, 0), (0, 0, 0)]);
        assert!(!v.draw(0));
    }
}
