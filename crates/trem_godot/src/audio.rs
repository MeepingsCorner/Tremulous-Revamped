//! Positional sound for game events: a small pool of AudioStreamPlayer3D
//! nodes (trap_S_StartSound) plus per-entity looping players
//! (trap_S_AddLoopingSound). Streams come from the Library (pk3 .wav).

use crate::geom::q2g;
use godot::classes::audio_stream_player_3d::AttenuationModel;
use godot::classes::{AudioStream, AudioStreamPlayer, AudioStreamPlayer3D};
use godot::prelude::*;

const POOL: usize = 48;

pub struct Audio {
    players: Vec<Gd<AudioStreamPlayer3D>>,
    local: Vec<Gd<AudioStreamPlayer>>,
    next: usize,
    next_local: usize,
}

pub fn new_3d() -> Gd<AudioStreamPlayer3D> {
    let mut p = AudioStreamPlayer3D::new_alloc();
    // Q3 sounds fade out around 1000-1500 units (~30-45 m at 32 units/m)
    p.set_unit_size(6.0);
    p.set_max_distance(48.0);
    p.set_attenuation_model(AttenuationModel::INVERSE_DISTANCE);
    p
}

impl Audio {
    /// The pool and the nodes the caller must add to the scene.
    pub fn new() -> (Self, Vec<Gd<Node>>) {
        let mut nodes: Vec<Gd<Node>> = Vec::new();
        let mut players = Vec::new();
        for _ in 0..POOL {
            let p = new_3d();
            nodes.push(p.clone().upcast());
            players.push(p);
        }
        let mut local = Vec::new();
        for _ in 0..8 {
            let p = AudioStreamPlayer::new_alloc();
            nodes.push(p.clone().upcast());
            local.push(p);
        }
        (Audio { players, local, next: 0, next_local: 0 }, nodes)
    }

    /// Positional one-shot at a Quake-space origin.
    pub fn play_at(&mut self, stream: &Gd<AudioStream>, origin: &[f32; 3], volume_db: f32) {
        // prefer an idle player, else steal round-robin
        let n = self.players.len();
        let mut idx = self.next;
        for k in 0..n {
            let j = (self.next + k) % n;
            if !self.players[j].is_playing() {
                idx = j;
                break;
            }
        }
        self.next = (idx + 1) % n;
        let p = &mut self.players[idx];
        p.set_stream(stream);
        p.set_volume_db(volume_db);
        p.set_global_position(q2g(origin));
        p.play();
    }

    /// Non-positional (the local player's own sounds, announcements).
    pub fn play_local(&mut self, stream: &Gd<AudioStream>, volume_db: f32) {
        let i = self.next_local;
        self.next_local = (i + 1) % self.local.len();
        let p = &mut self.local[i];
        p.set_stream(stream);
        p.set_volume_db(volume_db);
        p.play();
    }
}
