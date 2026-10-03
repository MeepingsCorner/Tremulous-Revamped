//! Kernel `v11net` — the wire of Tremulous 1.1.0 (protocol 69; dec040):
//! what re/source11's qcommon/msg.c, qcommon/common.c and
//! client/cl_net_chan.c do differently from legacy's. The netchan header
//! without the challenge checksum is `Netchan::protocol` (netchan.rs); the
//! client's protocol switch is in cl.rs; the game-data mapping (1.1
//! playerState / entityState / configstrings / commands to the legacy
//! client's) is v11.rs.
//!
//! * `entityStateFields` of 1.1: no `weaponAnim`, `powerups` (sent in the
//!   slot of legacy's `misc`, kept in `EntityState::misc`), `generic1` in 8 bits.
//! * `playerStateFields` of 1.1 over `PlayerState11`: no `weaponAnim`,
//!   `ammo`, `clips`, `tauntTimer`; `pm_flags` 16 bits, unsigned `gravity`
//!   and `speed`, `generic1` 8 bits, `jumppad_ent` where legacy has
//!   `otherEntityNum`; the arrays are stats, persistant, ammo (shorts) and
//!   powerups (longs).
//! * `MSG_ReadString` / `MSG_ReadBigString` / `MSG_ReadStringLine` turn
//!   '%' into '.' (only `MSG_ReadString` also drops 0x80+ chars).
//! * `Com_HashKey` (signed chars, no 0x80 rule) keys the usercmds.
//! * `CL_Netchan_Encode` / `CL_Netchan_Decode` xor the messages with the
//!   challenge, the sequence numbers and the last acknowledged command.
use super::msg::{set_word, word, EntityState, Msg, NetField, GENTITYNUM_BITS, MAX_STRING_CHARS, BIG_INFO_STRING};
use super::v11::PlayerState11;
use std::mem::offset_of;

/// `MAX_POWERUPS` / `MAX_WEAPONS` / `MAX_STATS` / `MAX_PERSISTANT` of 1.1.
pub const MAX_POWERUPS: usize = 16;
pub const MAX_WEAPONS: usize = 16;
pub const MAX_STATS: usize = 16;
pub const MAX_PERSISTANT: usize = 16;
/// `CL_ENCODE_START` / `CL_DECODE_START` (qcommon.h).
pub const CL_ENCODE_START: usize = 12;
pub const CL_DECODE_START: usize = 4;

macro_rules! esf {
    ($name:literal, $($f:ident).+ , $bits:expr) => {
        NetField { name: $name, offset: offset_of!(EntityState, $($f).+), bits: $bits }
    };
    ($name:literal, $($f:ident).+ [$i:literal], $bits:expr) => {
        NetField { name: $name, offset: offset_of!(EntityState, $($f).+) + 4 * $i, bits: $bits }
    };
}

/// 1.1.0's `entityStateFields` (msg.c), `powerups` read into `misc`.
pub const ENTITY_STATE_FIELDS_11: [NetField; 51] = [
    esf!("pos.trTime", pos.tr_time, 32),
    esf!("pos.trBase[0]", pos.tr_base[0], 0),
    esf!("pos.trBase[1]", pos.tr_base[1], 0),
    esf!("pos.trDelta[0]", pos.tr_delta[0], 0),
    esf!("pos.trDelta[1]", pos.tr_delta[1], 0),
    esf!("pos.trBase[2]", pos.tr_base[2], 0),
    esf!("apos.trBase[1]", apos.tr_base[1], 0),
    esf!("pos.trDelta[2]", pos.tr_delta[2], 0),
    esf!("apos.trBase[0]", apos.tr_base[0], 0),
    esf!("event", event, 10),
    esf!("angles2[1]", angles2[1], 0),
    esf!("eType", e_type, 8),
    esf!("torsoAnim", torso_anim, 8),
    esf!("eventParm", event_parm, 8),
    esf!("legsAnim", legs_anim, 8),
    esf!("groundEntityNum", ground_entity_num, GENTITYNUM_BITS),
    esf!("pos.trType", pos.tr_type, 8),
    esf!("eFlags", e_flags, 19),
    esf!("otherEntityNum", other_entity_num, GENTITYNUM_BITS),
    esf!("weapon", weapon, 8),
    esf!("clientNum", client_num, 8),
    esf!("angles[1]", angles[1], 0),
    esf!("pos.trDuration", pos.tr_duration, 32),
    esf!("apos.trType", apos.tr_type, 8),
    esf!("origin[0]", origin[0], 0),
    esf!("origin[1]", origin[1], 0),
    esf!("origin[2]", origin[2], 0),
    esf!("solid", solid, 24),
    esf!("powerups", misc, MAX_POWERUPS as i32),
    esf!("modelindex", modelindex, 8),
    esf!("otherEntityNum2", other_entity_num2, GENTITYNUM_BITS),
    esf!("loopSound", loop_sound, 8),
    esf!("generic1", generic1, 8),
    esf!("origin2[2]", origin2[2], 0),
    esf!("origin2[0]", origin2[0], 0),
    esf!("origin2[1]", origin2[1], 0),
    esf!("modelindex2", modelindex2, 8),
    esf!("angles[0]", angles[0], 0),
    esf!("time", time, 32),
    esf!("apos.trTime", apos.tr_time, 32),
    esf!("apos.trDuration", apos.tr_duration, 32),
    esf!("apos.trBase[2]", apos.tr_base[2], 0),
    esf!("apos.trDelta[0]", apos.tr_delta[0], 0),
    esf!("apos.trDelta[1]", apos.tr_delta[1], 0),
    esf!("apos.trDelta[2]", apos.tr_delta[2], 0),
    esf!("time2", time2, 32),
    esf!("angles[2]", angles[2], 0),
    esf!("angles2[0]", angles2[0], 0),
    esf!("angles2[2]", angles2[2], 0),
    esf!("constantLight", constant_light, 32),
    esf!("frame", frame, 16),
];

macro_rules! psf {
    ($name:literal, $f:ident, $bits:expr) => {
        NetField { name: $name, offset: offset_of!(PlayerState11, $f), bits: $bits }
    };
    ($name:literal, $f:ident [$i:literal], $bits:expr) => {
        NetField { name: $name, offset: offset_of!(PlayerState11, $f) + 4 * $i, bits: $bits }
    };
}

/// 1.1.0's `playerStateFields` (msg.c).
pub const PLAYER_STATE_FIELDS_11: [NetField; 48] = [
    psf!("commandTime", command_time, 32),
    psf!("origin[0]", origin[0], 0),
    psf!("origin[1]", origin[1], 0),
    psf!("bobCycle", bob_cycle, 8),
    psf!("velocity[0]", velocity[0], 0),
    psf!("velocity[1]", velocity[1], 0),
    psf!("viewangles[1]", viewangles[1], 0),
    psf!("viewangles[0]", viewangles[0], 0),
    psf!("weaponTime", weapon_time, -16),
    psf!("origin[2]", origin[2], 0),
    psf!("velocity[2]", velocity[2], 0),
    psf!("legsTimer", legs_timer, 8),
    psf!("pm_time", pm_time, -16),
    psf!("eventSequence", event_sequence, 16),
    psf!("torsoAnim", torso_anim, 8),
    psf!("movementDir", movement_dir, 4),
    psf!("events[0]", events[0], 8),
    psf!("legsAnim", legs_anim, 8),
    psf!("events[1]", events[1], 8),
    psf!("pm_flags", pm_flags, 16),
    psf!("groundEntityNum", ground_entity_num, GENTITYNUM_BITS),
    psf!("weaponstate", weaponstate, 4),
    psf!("eFlags", e_flags, 16),
    psf!("externalEvent", external_event, 10),
    psf!("gravity", gravity, 16),
    psf!("speed", speed, 16),
    psf!("delta_angles[1]", delta_angles[1], 16),
    psf!("externalEventParm", external_event_parm, 8),
    psf!("viewheight", viewheight, -8),
    psf!("damageEvent", damage_event, 8),
    psf!("damageYaw", damage_yaw, 8),
    psf!("damagePitch", damage_pitch, 8),
    psf!("damageCount", damage_count, 8),
    psf!("generic1", generic1, 8),
    psf!("pm_type", pm_type, 8),
    psf!("delta_angles[0]", delta_angles[0], 16),
    psf!("delta_angles[2]", delta_angles[2], 16),
    psf!("torsoTimer", torso_timer, 12),
    psf!("eventParms[0]", event_parms[0], 8),
    psf!("eventParms[1]", event_parms[1], 8),
    psf!("clientNum", client_num, 8),
    psf!("weapon", weapon, 5),
    psf!("viewangles[2]", viewangles[2], 0),
    psf!("grapplePoint[0]", grapple_point[0], 0),
    psf!("grapplePoint[1]", grapple_point[1], 0),
    psf!("grapplePoint[2]", grapple_point[2], 0),
    psf!("jumppad_ent", jumppad_ent, 10),
    psf!("loopSound", loop_sound, 16),
];

/// `Com_HashKey` (common.c of 1.1.0): `char` is signed and, unlike
/// legacy's `MSG_HashKey`, 0x80+ chars are hashed as they are.
pub fn com_hash_key(s: &[u8], maxlen: usize) -> i32 {
    let mut hash: i32 = 0;
    for (i, &c) in s.iter().enumerate().take(maxlen) {
        if c == 0 {
            break;
        }
        hash = hash.wrapping_add((c as i8 as i32).wrapping_mul(119 + i as i32));
    }
    hash ^ (hash >> 10) ^ (hash >> 20)
}

/// The byte of a C string at `i` (0 past its end).
fn cbyte(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// The key update of `CL_Netchan_Encode` / `CL_Netchan_Decode`: the last
/// acknowledged command's chars, '.' for 0x80+ and '%'.
fn netchan_xor(data: &mut [u8], from: usize, mut key: u8, string: &[u8]) {
    let mut index = 0usize;
    for i in from..data.len() {
        // modify the key with the last received now acknowledged server command
        if cbyte(string, index) == 0 {
            index = 0;
        }
        let c = cbyte(string, index);
        if c > 127 || c == b'%' {
            key ^= ((b'.' as u32) << (i & 1)) as u8;
        } else {
            key ^= ((c as u32) << (i & 1)) as u8;
        }
        index += 1;
        // encode the data with this key
        data[i] ^= key;
    }
}

/// `CL_Netchan_Encode`: the client message (before the netchan header;
/// its first 12 bytes are serverId, messageAcknowledge and
/// reliableAcknowledge) xored after `CL_ENCODE_START`. `server_commands`
/// is `clc.serverCommands`.
pub fn cl_netchan_encode(msg: &mut Msg, challenge: i32, server_commands: &[String]) {
    if msg.cursize <= CL_ENCODE_START {
        return;
    }
    let (srdc, sbit, soob) = (msg.readcount, msg.bit, msg.oob);
    msg.bit = 0;
    msg.readcount = 0;
    msg.oob = false;
    let server_id = msg.read_long();
    let message_acknowledge = msg.read_long();
    let reliable_acknowledge = msg.read_long();
    msg.oob = soob;
    msg.bit = sbit;
    msg.readcount = srdc;
    // index the client's own ring (128 slots, sv::MAX_RELIABLE_COMMANDS):
    // masking with 1.1's 64 picked the wrong command once 64 were acknowledged
    let string = server_commands[reliable_acknowledge as usize & (server_commands.len() - 1)].as_bytes();
    let key = (challenge ^ server_id ^ message_acknowledge) as u8;
    let n = msg.cursize;
    netchan_xor(&mut msg.data[..n], CL_ENCODE_START, key, string);
}

/// `CL_Netchan_Decode`: a server message after `Netchan_Process` (the
/// reader past the header; its first long is reliableAcknowledge), xored
/// after `CL_DECODE_START`. `reliable_commands` is `clc.reliableCommands`.
pub fn cl_netchan_decode(msg: &mut Msg, challenge: i32, reliable_commands: &[String]) {
    let (srdc, sbit, soob) = (msg.readcount, msg.bit, msg.oob);
    msg.oob = false;
    let reliable_acknowledge = msg.read_long();
    msg.oob = soob;
    msg.bit = sbit;
    msg.readcount = srdc;
    // the client's ring has 128 slots (sv::MAX_RELIABLE_COMMANDS): masking with
    // 1.1's 64 decoded every message with the wrong command string once the
    // server had acknowledged 64 commands (UDP downloads died ~100 blocks in)
    let string = reliable_commands[reliable_acknowledge as usize & (reliable_commands.len() - 1)].as_bytes();
    // xor the client challenge with the netchan sequence number (need something that changes every message)
    let seq = i32::from_le_bytes([msg.data[0], msg.data[1], msg.data[2], msg.data[3]]);
    let key = (challenge ^ seq) as u8;
    let n = msg.cursize.min(msg.data.len());
    if msg.readcount + CL_DECODE_START < n {
        netchan_xor(&mut msg.data[..n], msg.readcount + CL_DECODE_START, key, string);
    }
}

impl Msg {
    fn read_string_11(&mut self, max: usize, newline: bool, high: bool) -> String {
        let mut s = Vec::new();
        loop {
            let c = self.read_byte(); // use ReadByte so -1 is out of bounds
            if c == -1 || c == 0 || (newline && c == b'\n' as i32) {
                break;
            }
            // translate all fmt spec to avoid crash bugs
            let mut c = if c == b'%' as i32 { b'.' as i32 } else { c };
            // don't allow higher ascii values
            if high && c > 127 {
                c = b'.' as i32;
            }
            s.push(c as u8);
            if s.len() >= max - 1 {
                break;
            }
        }
        String::from_utf8_lossy(&s).into_owned()
    }
    /// 1.1.0's `MSG_ReadString`.
    pub fn read_string11(&mut self) -> String {
        self.read_string_11(MAX_STRING_CHARS, false, true)
    }
    /// 1.1.0's `MSG_ReadBigString`.
    pub fn read_big_string11(&mut self) -> String {
        self.read_string_11(BIG_INFO_STRING, false, false)
    }
    /// 1.1.0's `MSG_ReadStringLine`.
    pub fn read_string_line11(&mut self) -> String {
        self.read_string_11(MAX_STRING_CHARS, true, false)
    }

    /// 1.1.0's `MSG_WriteDeltaEntity`.
    pub fn write_delta_entity11(&mut self, from: Option<&EntityState>, to: Option<&EntityState>, force: bool) {
        self.write_delta_entity_fields(&ENTITY_STATE_FIELDS_11, from, to, force)
    }

    /// 1.1.0's `MSG_ReadDeltaEntity`: a 1.1 entityState (powerups in `misc`,
    /// no weaponAnim); a removed entity comes back with number MAX_GENTITIES-1.
    pub fn read_delta_entity11(&mut self, from: &EntityState, number: i32) -> EntityState {
        self.read_delta_entity_fields(&ENTITY_STATE_FIELDS_11, from, number)
    }

    /// 1.1.0's `MSG_WriteDeltaPlayerstate` (`from` None: a zeroed state).
    pub fn write_delta_playerstate11(&mut self, from: Option<&PlayerState11>, to: &PlayerState11) {
        let dummy = PlayerState11::default();
        let from = from.unwrap_or(&dummy);
        let mut lc = 0;
        for (i, f) in PLAYER_STATE_FIELDS_11.iter().enumerate() {
            if word(from, f.offset) != word(to, f.offset) {
                lc = i + 1;
            }
        }
        self.write_byte(lc as i32); // # of changes
        for f in &PLAYER_STATE_FIELDS_11[..lc] {
            let (fv, tv) = (word(from, f.offset), word(to, f.offset));
            if fv == tv {
                self.write_bits(0, 1); // no change
                continue;
            }
            self.write_bits(1, 1); // changed
            if f.bits == 0 {
                self.write_net_float(tv, false);
            } else {
                // integer
                self.write_bits(tv, f.bits);
            }
        }
        // send the arrays
        let bits_of = |a: &[i32], b: &[i32]| -> i32 { (0..a.len()).filter(|&i| a[i] != b[i]).fold(0, |m, i| m | 1 << i) };
        let statsbits = bits_of(&to.stats, &from.stats);
        let persistantbits = bits_of(&to.persistant, &from.persistant);
        let ammobits = bits_of(&to.ammo, &from.ammo);
        let powerupbits = bits_of(&to.powerups, &from.powerups);
        if statsbits == 0 && persistantbits == 0 && ammobits == 0 && powerupbits == 0 {
            self.write_bits(0, 1); // no change
            return;
        }
        self.write_bits(1, 1); // changed
        for (bits, arr, n, long) in [(statsbits, &to.stats, MAX_STATS, false), (persistantbits, &to.persistant, MAX_PERSISTANT, false), (ammobits, &to.ammo, MAX_WEAPONS, false), (powerupbits, &to.powerups, MAX_POWERUPS, true)] {
            if bits != 0 {
                self.write_bits(1, 1); // changed
                self.write_bits(bits, n as i32);
                for (i, &v) in arr.iter().enumerate().take(n) {
                    if bits & (1 << i) != 0 {
                        if long {
                            self.write_long(v);
                        } else {
                            self.write_short(v);
                        }
                    }
                }
            } else {
                self.write_bits(0, 1); // no change
            }
        }
    }

    /// 1.1.0's `MSG_ReadDeltaPlayerstate`.
    pub fn read_delta_playerstate11(&mut self, from: Option<&PlayerState11>) -> PlayerState11 {
        let dummy = PlayerState11::default();
        let from = from.unwrap_or(&dummy);
        let mut to = *from;
        let mut lc = self.read_byte();
        // 1.1.0 has no "invalid playerState field count" check; a count
        // past the table would read past it: keep what parses
        if lc > PLAYER_STATE_FIELDS_11.len() as i32 || lc < 0 {
            lc = PLAYER_STATE_FIELDS_11.len() as i32;
        }
        for f in &PLAYER_STATE_FIELDS_11[..lc as usize] {
            if self.read_bits(1) == 0 {
                // no change
                continue;
            }
            let v = if f.bits == 0 { self.read_net_float() } else { self.read_bits(f.bits) };
            set_word(&mut to, f.offset, v);
        }
        // read the arrays
        if self.read_bits(1) != 0 {
            // parse stats, persistant stats, ammo (shorts), powerups (longs)
            for k in 0..4 {
                if self.read_bits(1) == 0 {
                    continue;
                }
                let n = [MAX_STATS, MAX_PERSISTANT, MAX_WEAPONS, MAX_POWERUPS][k];
                let bits = self.read_bits(n as i32);
                for i in 0..n {
                    if bits & (1 << i) != 0 {
                        let v = if k == 3 { self.read_long() } else { self.read_short() };
                        match k {
                            0 => to.stats[i] = v,
                            1 => to.persistant[i] = v,
                            2 => to.ammo[i] = v,
                            _ => to.powerups[i] = v,
                        }
                    }
                }
            }
        }
        to
    }
}
