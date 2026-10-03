//! Kernel `msg` — qcommon/msg.c: the message buffer of the network layer.
//! Out-of-band messages are plain little-endian bytes; netchan messages are
//! a bit stream whose whole bytes go through the fixed Huffman code of
//! `MSG_initHuffman` (huffman.rs). On top of the bit functions: strings,
//! the keyed `usercmd_t` delta (`MSG_WriteDeltaUsercmdKey`), and the field
//! tables `entityStateFields` / `playerStateFields` that drive
//! `MSG_WriteDeltaEntity` / `MSG_WriteDeltaPlayerstate` (fields addressed by
//! their offset in the struct, as the C's `NETF` / `PSF` do).

use super::bg::Trajectory;
use super::huffman::{self, msg_huff};
use super::qmath::Vec3;
use super::types::{PlayerState, UserCmd, MAX_PERSISTANT, MAX_STATS};
use std::mem::offset_of;

/// `MAX_MSGLEN`: max length of a message, which may be fragmented.
pub const MAX_MSGLEN: usize = 16384;
/// `MAX_STRING_CHARS`.
pub const MAX_STRING_CHARS: usize = 1024;
/// `BIG_INFO_STRING`.
pub const BIG_INFO_STRING: usize = 8192;
/// `GENTITYNUM_BITS` / `MAX_GENTITIES`.
pub const GENTITYNUM_BITS: i32 = 10;
pub const MAX_GENTITIES: i32 = 1 << GENTITYNUM_BITS;
/// `MAX_MISC`.
pub const MAX_MISC: usize = 16;
/// `FLOAT_INT_BITS` / `FLOAT_INT_BIAS`.
pub const FLOAT_INT_BITS: i32 = 13;
pub const FLOAT_INT_BIAS: i32 = 1 << (FLOAT_INT_BITS - 1);

/// `msg_t`.
#[derive(Clone, Debug, Default)]
pub struct Msg {
    pub allowoverflow: bool,
    pub overflowed: bool,
    pub oob: bool,
    /// the buffer (`data`, `maxsize` = its length)
    pub data: Vec<u8>,
    pub cursize: usize,
    pub readcount: usize,
    /// for bitwise reads and writes
    pub bit: usize,
}

/// `kbitmask`.
const KBITMASK: [u32; 32] = {
    let mut a = [0u32; 32];
    let mut i = 0;
    while i < 32 {
        a[i] = if i == 31 { 0xffff_ffff } else { (1u32 << (i + 1)) - 1 };
        i += 1;
    }
    a
};

impl Msg {
    /// `MSG_Init`.
    pub fn new(maxsize: usize) -> Self {
        let _ = msg_huff();
        Msg { data: vec![0; maxsize], ..Default::default() }
    }
    /// `MSG_InitOOB`.
    pub fn new_oob(maxsize: usize) -> Self {
        Msg { oob: true, ..Self::new(maxsize) }
    }
    /// A received packet: `MSG_Init` over a `MAX_MSGLEN` buffer holding `bytes`.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut m = Self::new(MAX_MSGLEN.max(bytes.len()));
        m.data[..bytes.len()].copy_from_slice(bytes);
        m.cursize = bytes.len();
        m
    }
    pub fn maxsize(&self) -> usize {
        self.data.len()
    }
    /// The written bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.cursize.min(self.data.len())]
    }
    /// `MSG_Clear`.
    pub fn clear(&mut self) {
        self.cursize = 0;
        self.overflowed = false;
        self.bit = 0;
    }
    /// `MSG_Bitstream`.
    pub fn bitstream(&mut self) {
        self.oob = false;
    }
    /// `MSG_BeginReading`.
    pub fn begin_reading(&mut self) {
        self.readcount = 0;
        self.bit = 0;
        self.oob = false;
    }
    /// `MSG_BeginReadingOOB`.
    pub fn begin_reading_oob(&mut self) {
        self.readcount = 0;
        self.bit = 0;
        self.oob = true;
    }

    // ------------------------------------------------------------ bits
    /// `MSG_WriteBits` (negative bit counts include signs).
    pub fn write_bits(&mut self, value: i32, bits: i32) {
        // this isn't an exact overflow check, but close enough
        if self.maxsize().saturating_sub(self.cursize) < 4 {
            self.overflowed = true;
            return;
        }
        assert!(bits != 0 && (-31..=32).contains(&bits), "MSG_WriteBits: bad bits {bits}");
        let bits = bits.unsigned_abs() as usize;
        if self.oob {
            let c = self.cursize;
            match bits {
                8 => {
                    self.data[c] = value as u8;
                    self.cursize += 1;
                }
                16 => {
                    self.data[c..c + 2].copy_from_slice(&(value as i16).to_le_bytes());
                    self.cursize += 2;
                }
                32 => {
                    self.data[c..c + 4].copy_from_slice(&value.to_le_bytes());
                    self.cursize += 4;
                }
                _ => panic!("can't write {bits} bits"),
            }
            self.bit += bits;
            return;
        }
        let mut value = value as u32 & (0xffff_ffffu32 >> (32 - bits));
        let mut bits = bits;
        let h = msg_huff();
        let mut w = huffman::BitWriter { buf: &mut self.data, bloc: self.bit };
        if bits & 7 != 0 {
            let nbits = bits & 7;
            for _ in 0..nbits {
                w.add_bit((value & 1) as u8);
                value >>= 1;
            }
            bits -= nbits;
        }
        let mut i = 0;
        while i < bits {
            // Huff_offsetTransmit
            for &b in &h.codes[(value & 0xff) as usize] {
                w.add_bit(b);
            }
            value >>= 8;
            i += 8;
        }
        self.bit = w.bloc;
        self.cursize = (self.bit >> 3) + 1;
    }

    /// `MSG_ReadBits`.
    pub fn read_bits(&mut self, bits: i32) -> i32 {
        let sgn = bits < 0;
        let mut bits = bits.unsigned_abs() as usize;
        let mut value: u32 = 0;
        if self.oob {
            let r = self.readcount;
            let byte = |i: usize| self.data.get(i).copied().unwrap_or(0);
            match bits {
                8 => {
                    value = byte(r) as u32;
                    self.readcount += 1;
                }
                16 => {
                    value = i16::from_le_bytes([byte(r), byte(r + 1)]) as i32 as u32;
                    self.readcount += 2;
                }
                32 => {
                    value = u32::from_le_bytes([byte(r), byte(r + 1), byte(r + 2), byte(r + 3)]);
                    self.readcount += 4;
                }
                _ => panic!("can't read {bits} bits"),
            }
            self.bit += bits;
        } else {
            let mut nbits = 0;
            if bits & 7 != 0 {
                nbits = bits & 7;
                for i in 0..nbits {
                    value |= (huffman::get_bit(&self.data, &mut self.bit) as u32) << i;
                }
                bits -= nbits;
            }
            let h = msg_huff();
            let mut i = 0;
            while i < bits {
                // Huff_offsetReceive
                let data = &self.data;
                let bit = &mut self.bit;
                let get = h.tree.receive(&mut || huffman::get_bit(data, bit)) as u32;
                value |= get.wrapping_shl((i + nbits) as u32);
                i += 8;
            }
            self.readcount = (self.bit >> 3) + 1;
        }
        let mut v = value as i32;
        if sgn && bits > 0 && bits < 32 && v & (1 << (bits - 1)) != 0 {
            v |= -1 ^ ((1 << bits) - 1);
        }
        v
    }

    // ------------------------------------------------------------ writing
    /// `MSG_WriteChar`.
    pub fn write_char(&mut self, c: i32) {
        self.write_bits(c, 8);
    }
    /// `MSG_WriteByte`.
    pub fn write_byte(&mut self, c: i32) {
        self.write_bits(c, 8);
    }
    /// `MSG_WriteData`.
    pub fn write_data(&mut self, data: &[u8]) {
        for &b in data {
            self.write_byte(b as i32);
        }
    }
    /// `MSG_WriteShort`.
    pub fn write_short(&mut self, c: i32) {
        self.write_bits(c, 16);
    }
    /// `MSG_WriteLong`.
    pub fn write_long(&mut self, c: i32) {
        self.write_bits(c, 32);
    }
    /// `MSG_WriteFloat`.
    pub fn write_float(&mut self, f: f32) {
        self.write_bits(f.to_bits() as i32, 32);
    }
    /// `MSG_WriteString` over raw C-string bytes (no NUL inside).
    pub fn write_string_bytes(&mut self, b: &[u8]) {
        self.write_bytes_max(b, MAX_STRING_CHARS);
    }
    fn write_string_max(&mut self, s: &str, max: usize) {
        self.write_bytes_max(s.as_bytes(), max);
    }
    fn write_bytes_max(&mut self, b: &[u8], max: usize) {
        if b.len() >= max {
            self.write_data(&[0]);
            return;
        }
        // get rid of 0x80+ chars, because old clients don't like them
        let mut v: Vec<u8> = b.iter().map(|&c| if c > 127 { b'.' } else { c }).collect();
        v.push(0);
        self.write_data(&v);
    }
    /// `MSG_WriteString`.
    pub fn write_string(&mut self, s: &str) {
        self.write_string_max(s, MAX_STRING_CHARS);
    }
    /// `MSG_WriteBigString`.
    pub fn write_big_string(&mut self, s: &str) {
        self.write_string_max(s, BIG_INFO_STRING);
    }
    /// `MSG_WriteAngle`.
    pub fn write_angle(&mut self, f: f32) {
        self.write_byte((f * 256.0 / 360.0) as i32 & 255);
    }
    /// `MSG_WriteAngle16`.
    pub fn write_angle16(&mut self, f: f32) {
        self.write_short(angle2short(f));
    }

    // ------------------------------------------------------------ reading
    /// `MSG_ReadChar` (-1 past the end).
    pub fn read_char(&mut self) -> i32 {
        let c = self.read_bits(8) as i8 as i32;
        if self.readcount > self.cursize {
            -1
        } else {
            c
        }
    }
    /// `MSG_ReadByte` (-1 past the end).
    pub fn read_byte(&mut self) -> i32 {
        let c = self.read_bits(8) as u8 as i32;
        if self.readcount > self.cursize {
            -1
        } else {
            c
        }
    }
    /// `MSG_LookaheadByte`.
    pub fn lookahead_byte(&mut self) -> i32 {
        let (r, b) = (self.readcount, self.bit);
        let c = self.read_byte();
        self.readcount = r;
        self.bit = b;
        c
    }
    /// `MSG_ReadShort` (-1 past the end).
    pub fn read_short(&mut self) -> i32 {
        let c = self.read_bits(16) as i16 as i32;
        if self.readcount > self.cursize {
            -1
        } else {
            c
        }
    }
    /// `MSG_ReadLong` (-1 past the end).
    pub fn read_long(&mut self) -> i32 {
        let c = self.read_bits(32);
        if self.readcount > self.cursize {
            -1
        } else {
            c
        }
    }
    /// `MSG_ReadFloat`.
    pub fn read_float(&mut self) -> f32 {
        let i = self.read_bits(32);
        if self.readcount > self.cursize {
            -1.0
        } else {
            f32::from_bits(i as u32)
        }
    }
    fn read_string_until(&mut self, max: usize, newline: bool) -> String {
        let mut s = Vec::new();
        loop {
            let c = self.read_byte(); // use ReadByte so -1 is out of bounds
            if c == -1 || c == 0 || (newline && c == b'\n' as i32) {
                break;
            }
            // don't allow higher ascii values
            s.push(if c > 127 { b'.' } else { c as u8 });
            if s.len() >= max - 1 {
                break;
            }
        }
        String::from_utf8_lossy(&s).into_owned()
    }
    /// `MSG_ReadString`.
    pub fn read_string(&mut self) -> String {
        self.read_string_until(MAX_STRING_CHARS, false)
    }
    /// `MSG_ReadBigString`.
    pub fn read_big_string(&mut self) -> String {
        self.read_string_until(BIG_INFO_STRING, false)
    }
    /// `MSG_ReadStringLine`.
    pub fn read_string_line(&mut self) -> String {
        self.read_string_until(MAX_STRING_CHARS, true)
    }
    /// `MSG_ReadAngle16`.
    pub fn read_angle16(&mut self) -> f32 {
        short2angle(self.read_short())
    }
    /// `MSG_ReadData`.
    pub fn read_data(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.read_byte() as u8).collect()
    }

    // ------------------------------------------------------------ deltas
    /// `MSG_WriteDelta`.
    pub fn write_delta(&mut self, old: i32, new: i32, bits: i32) {
        if old == new {
            self.write_bits(0, 1);
            return;
        }
        self.write_bits(1, 1);
        self.write_bits(new, bits);
    }
    /// `MSG_ReadDelta`.
    pub fn read_delta(&mut self, old: i32, bits: i32) -> i32 {
        if self.read_bits(1) != 0 {
            return self.read_bits(bits);
        }
        old
    }
    /// `MSG_WriteDeltaFloat`.
    pub fn write_delta_float(&mut self, old: f32, new: f32) {
        if old == new {
            self.write_bits(0, 1);
            return;
        }
        self.write_bits(1, 1);
        self.write_bits(new.to_bits() as i32, 32);
    }
    /// `MSG_ReadDeltaFloat`.
    pub fn read_delta_float(&mut self, old: f32) -> f32 {
        if self.read_bits(1) != 0 {
            return f32::from_bits(self.read_bits(32) as u32);
        }
        old
    }
    /// `MSG_WriteDeltaKey`.
    pub fn write_delta_key(&mut self, key: i32, old: i32, new: i32, bits: i32) {
        if old == new {
            self.write_bits(0, 1);
            return;
        }
        self.write_bits(1, 1);
        self.write_bits(new ^ key, bits);
    }
    /// `MSG_ReadDeltaKey`.
    pub fn read_delta_key(&mut self, key: i32, old: i32, bits: i32) -> i32 {
        if self.read_bits(1) != 0 {
            return self.read_bits(bits) ^ (key & KBITMASK[bits as usize - 1] as i32);
        }
        old
    }
    /// `MSG_WriteDeltaKeyFloat`.
    pub fn write_delta_key_float(&mut self, key: i32, old: f32, new: f32) {
        if old == new {
            self.write_bits(0, 1);
            return;
        }
        self.write_bits(1, 1);
        self.write_bits(new.to_bits() as i32 ^ key, 32);
    }
    /// `MSG_ReadDeltaKeyFloat`.
    pub fn read_delta_key_float(&mut self, key: i32, old: f32) -> f32 {
        if self.read_bits(1) != 0 {
            return f32::from_bits((self.read_bits(32) ^ key) as u32);
        }
        old
    }

    /// `MSG_WriteDeltaUsercmdKey`.
    pub fn write_delta_usercmd_key(&mut self, key: i32, from: &UserCmd, to: &UserCmd) {
        if to.server_time.wrapping_sub(from.server_time) < 256 {
            self.write_bits(1, 1);
            self.write_bits(to.server_time.wrapping_sub(from.server_time), 8);
        } else {
            self.write_bits(0, 1);
            self.write_bits(to.server_time, 32);
        }
        if from.angles == to.angles && from.forwardmove == to.forwardmove && from.rightmove == to.rightmove && from.upmove == to.upmove && from.buttons == to.buttons && from.weapon == to.weapon {
            self.write_bits(0, 1); // no change
            return;
        }
        let key = key ^ to.server_time;
        self.write_bits(1, 1);
        for i in 0..3 {
            self.write_delta_key(key, from.angles[i], to.angles[i], 16);
        }
        self.write_delta_key(key, from.forwardmove as i32, to.forwardmove as i32, 8);
        self.write_delta_key(key, from.rightmove as i32, to.rightmove as i32, 8);
        self.write_delta_key(key, from.upmove as i32, to.upmove as i32, 8);
        self.write_delta_key(key, from.buttons, to.buttons, 16);
        self.write_delta_key(key, from.weapon as i32, to.weapon as i32, 8);
    }

    /// `MSG_ReadDeltaUsercmdKey`.
    pub fn read_delta_usercmd_key(&mut self, key: i32, from: &UserCmd) -> UserCmd {
        let mut to = UserCmd::default();
        if self.read_bits(1) != 0 {
            to.server_time = from.server_time.wrapping_add(self.read_bits(8));
        } else {
            to.server_time = self.read_bits(32);
        }
        if self.read_bits(1) != 0 {
            let key = key ^ to.server_time;
            for i in 0..3 {
                // usercmd angles are ints in the C; the delta keeps 16 bits
                to.angles[i] = self.read_delta_key(key, from.angles[i], 16);
            }
            let sc = |v: i32| -> i8 {
                let c = v as u8 as i8;
                if c == -128 {
                    -127
                } else {
                    c
                }
            };
            to.forwardmove = sc(self.read_delta_key(key, from.forwardmove as i32, 8));
            to.rightmove = sc(self.read_delta_key(key, from.rightmove as i32, 8));
            to.upmove = sc(self.read_delta_key(key, from.upmove as i32, 8));
            to.buttons = self.read_delta_key(key, from.buttons, 16);
            to.weapon = self.read_delta_key(key, from.weapon as i32, 8) as u8;
        } else {
            to.angles = from.angles;
            to.forwardmove = from.forwardmove;
            to.rightmove = from.rightmove;
            to.upmove = from.upmove;
            to.buttons = from.buttons;
            to.weapon = from.weapon;
        }
        to
    }
}

/// `MSG_HashKey`: a string hash that gives the same value even after the
/// legacy MSG read/write code turned 0x80+ chars into '.'.
pub fn hash_key(s: &str, maxlen: usize) -> i32 {
    let mut hash: i32 = 0;
    for (i, &c) in s.as_bytes().iter().enumerate().take(maxlen) {
        if c == 0 {
            break;
        }
        let c = if c & 0x80 != 0 { b'.' as i32 } else { c as i32 };
        hash = hash.wrapping_add(c.wrapping_mul(119 + i as i32));
    }
    hash ^ (hash >> 10) ^ (hash >> 20)
}

/// `ANGLE2SHORT`.
pub fn angle2short(x: f32) -> i32 {
    ((x * 65536.0 / 360.0) as i32) & 65535
}
/// `SHORT2ANGLE`.
pub fn short2angle(x: i32) -> f32 {
    (x as f64 * (360.0 / 65536.0)) as f32
}

// =================================================================== entityState_t

/// `entityState_t` (q_shared.h), field for field. Every member is 32 bits,
/// so the netField tables can address them by offset.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EntityState {
    pub number: i32,
    pub e_type: i32,
    pub e_flags: i32,
    pub pos: Trajectory,
    pub apos: Trajectory,
    pub time: i32,
    pub time2: i32,
    pub origin: Vec3,
    pub origin2: Vec3,
    pub angles: Vec3,
    pub angles2: Vec3,
    pub other_entity_num: i32,
    pub other_entity_num2: i32,
    pub ground_entity_num: i32,
    pub constant_light: i32,
    pub loop_sound: i32,
    pub modelindex: i32,
    pub modelindex2: i32,
    pub client_num: i32,
    pub frame: i32,
    pub solid: i32,
    pub event: i32,
    pub event_parm: i32,
    pub misc: i32,
    pub weapon: i32,
    pub legs_anim: i32,
    pub torso_anim: i32,
    pub weapon_anim: i32,
    pub generic1: i32,
}

/// `netField_t`: name, byte offset, bits (0 = float, negative = signed).
#[derive(Clone, Copy, Debug)]
pub struct NetField {
    pub name: &'static str,
    pub offset: usize,
    pub bits: i32,
}

macro_rules! esf {
    ($name:literal, $($f:ident).+ , $bits:expr) => {
        NetField { name: $name, offset: offset_of!(EntityState, $($f).+), bits: $bits }
    };
    ($name:literal, $($f:ident).+ [$i:literal], $bits:expr) => {
        NetField { name: $name, offset: offset_of!(EntityState, $($f).+) + 4 * $i, bits: $bits }
    };
}

/// `entityStateFields` in the C's (transmission-frequency) order.
pub const ENTITY_STATE_FIELDS: [NetField; 52] = [
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
    esf!("weaponAnim", weapon_anim, 8),
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
    esf!("misc", misc, MAX_MISC as i32),
    esf!("modelindex", modelindex, 8),
    esf!("otherEntityNum2", other_entity_num2, GENTITYNUM_BITS),
    esf!("loopSound", loop_sound, 8),
    esf!("generic1", generic1, 10),
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
        NetField { name: $name, offset: offset_of!(PlayerState, $f), bits: $bits }
    };
    ($name:literal, $f:ident [$i:literal], $bits:expr) => {
        NetField { name: $name, offset: offset_of!(PlayerState, $f) + 4 * $i, bits: $bits }
    };
}

/// `playerStateFields`.
pub const PLAYER_STATE_FIELDS: [NetField; 52] = [
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
    psf!("weaponAnim", weapon_anim, 8),
    psf!("movementDir", movement_dir, 4),
    psf!("events[0]", events[0], 8),
    psf!("legsAnim", legs_anim, 8),
    psf!("events[1]", events[1], 8),
    psf!("pm_flags", pm_flags, 24),
    psf!("groundEntityNum", ground_entity_num, GENTITYNUM_BITS),
    psf!("weaponstate", weaponstate, 4),
    psf!("eFlags", e_flags, 16),
    psf!("externalEvent", external_event, 10),
    psf!("gravity", gravity, -16),
    psf!("speed", speed, -16),
    psf!("delta_angles[1]", delta_angles[1], 16),
    psf!("externalEventParm", external_event_parm, 8),
    psf!("viewheight", viewheight, -8),
    psf!("damageEvent", damage_event, 8),
    psf!("damageYaw", damage_yaw, 8),
    psf!("damagePitch", damage_pitch, 8),
    psf!("damageCount", damage_count, 8),
    psf!("ammo", ammo, 12),
    psf!("clips", clips, 4),
    psf!("generic1", generic1, 10),
    psf!("pm_type", pm_type, 8),
    psf!("delta_angles[0]", delta_angles[0], 16),
    psf!("delta_angles[2]", delta_angles[2], 16),
    psf!("torsoTimer", torso_timer, 12),
    psf!("tauntTimer", taunt_timer, 12),
    psf!("eventParms[0]", event_parms[0], 8),
    psf!("eventParms[1]", event_parms[1], 8),
    psf!("clientNum", client_num, 8),
    psf!("weapon", weapon, 5),
    psf!("viewangles[2]", viewangles[2], 0),
    psf!("grapplePoint[0]", grapple_point[0], 0),
    psf!("grapplePoint[1]", grapple_point[1], 0),
    psf!("grapplePoint[2]", grapple_point[2], 0),
    psf!("otherEntityNum", other_entity_num, GENTITYNUM_BITS),
    psf!("loopSound", loop_sound, 16),
];

/// The 32-bit word at `offset` of a struct made only of 32-bit members
/// (`*(int *)( (byte *)from + field->offset )`).
#[inline]
pub(crate) fn word<T>(s: &T, offset: usize) -> i32 {
    debug_assert!(offset + 4 <= std::mem::size_of::<T>() && offset % 4 == 0);
    // SAFETY: the tables only hold offsets of i32 / f32 members (offset_of!).
    unsafe { std::ptr::read((s as *const T as *const u8).add(offset) as *const i32) }
}
#[inline]
pub(crate) fn set_word<T>(s: &mut T, offset: usize, v: i32) {
    debug_assert!(offset + 4 <= std::mem::size_of::<T>() && offset % 4 == 0);
    // SAFETY: as in `word`.
    unsafe { std::ptr::write((s as *mut T as *mut u8).add(offset) as *mut i32, v) }
}

impl Msg {
    /// A float field: 0 bit for 0.0 (entities only), else a small integer in
    /// FLOAT_INT_BITS or the full 32 bits.
    pub(crate) fn write_net_float(&mut self, bits: i32, zero_bit: bool) {
        let full = f32::from_bits(bits as u32);
        let trunc = full as i32;
        if zero_bit {
            if full == 0.0 {
                self.write_bits(0, 1);
                return;
            }
            self.write_bits(1, 1);
        }
        if trunc as f32 == full && trunc + FLOAT_INT_BIAS >= 0 && trunc + FLOAT_INT_BIAS < (1 << FLOAT_INT_BITS) {
            // send as small integer
            self.write_bits(0, 1);
            self.write_bits(trunc + FLOAT_INT_BIAS, FLOAT_INT_BITS);
        } else {
            // send as full floating point value
            self.write_bits(1, 1);
            self.write_bits(bits, 32);
        }
    }
    pub(crate) fn read_net_float(&mut self) -> i32 {
        if self.read_bits(1) == 0 {
            // integral float
            let trunc = self.read_bits(FLOAT_INT_BITS) - FLOAT_INT_BIAS;
            (trunc as f32).to_bits() as i32
        } else {
            self.read_bits(32)
        }
    }

    /// `MSG_WriteDeltaEntity`: `to` None is a remove; with `force` false an
    /// unchanged entity writes nothing.
    pub fn write_delta_entity(&mut self, from: Option<&EntityState>, to: Option<&EntityState>, force: bool) {
        self.write_delta_entity_fields(&ENTITY_STATE_FIELDS, from, to, force)
    }

    /// `MSG_WriteDeltaEntity` over a protocol's `entityStateFields`.
    pub(crate) fn write_delta_entity_fields(&mut self, fields: &[NetField], from: Option<&EntityState>, to: Option<&EntityState>, force: bool) {
        let Some(to) = to else {
            if let Some(from) = from {
                self.write_bits(from.number, GENTITYNUM_BITS);
                self.write_bits(1, 1);
            }
            return;
        };
        assert!(to.number >= 0 && to.number < MAX_GENTITIES, "MSG_WriteDeltaEntity: Bad entity number: {}", to.number);
        let null = EntityState::default();
        let from = from.unwrap_or(&null);
        let mut lc = 0;
        // build the change vector as bytes so it is endien independent
        for (i, f) in fields.iter().enumerate() {
            if word(from, f.offset) != word(to, f.offset) {
                lc = i + 1;
            }
        }
        if lc == 0 {
            // nothing at all changed
            if !force {
                return;
            }
            // write two bits for no change
            self.write_bits(to.number, GENTITYNUM_BITS);
            self.write_bits(0, 1); // not removed
            self.write_bits(0, 1); // no delta
            return;
        }
        self.write_bits(to.number, GENTITYNUM_BITS);
        self.write_bits(0, 1); // not removed
        self.write_bits(1, 1); // we have a delta
        self.write_byte(lc as i32); // # of changes
        for f in &fields[..lc] {
            let (fv, tv) = (word(from, f.offset), word(to, f.offset));
            if fv == tv {
                self.write_bits(0, 1); // no change
                continue;
            }
            self.write_bits(1, 1); // changed
            if f.bits == 0 {
                self.write_net_float(tv, true);
            } else if tv == 0 {
                self.write_bits(0, 1);
            } else {
                self.write_bits(1, 1);
                // integer
                self.write_bits(tv, f.bits);
            }
        }
    }

    /// `MSG_ReadDeltaEntity`: the number was already read. A removed entity
    /// comes back with number MAX_GENTITIES-1.
    pub fn read_delta_entity(&mut self, from: &EntityState, number: i32) -> EntityState {
        self.read_delta_entity_fields(&ENTITY_STATE_FIELDS, from, number)
    }

    /// `MSG_ReadDeltaEntity` over a protocol's `entityStateFields`.
    pub(crate) fn read_delta_entity_fields(&mut self, fields: &[NetField], from: &EntityState, number: i32) -> EntityState {
        assert!((0..MAX_GENTITIES).contains(&number), "Bad delta entity number: {number}");
        // check for a remove
        if self.read_bits(1) == 1 {
            return EntityState { number: MAX_GENTITIES - 1, ..Default::default() };
        }
        // check for no delta
        if self.read_bits(1) == 0 {
            return EntityState { number, ..*from };
        }
        let mut lc = self.read_byte();
        if lc > fields.len() as i32 || lc < 0 {
            // ERR_DROP "invalid entityState field count": keep what parses
            lc = fields.len() as i32;
        }
        let mut to = *from;
        to.number = number;
        for f in &fields[..lc as usize] {
            if self.read_bits(1) == 0 {
                // no change
                continue;
            }
            let v = if f.bits == 0 {
                if self.read_bits(1) == 0 {
                    0f32.to_bits() as i32
                } else {
                    self.read_net_float()
                }
            } else if self.read_bits(1) == 0 {
                0
            } else {
                // integer
                self.read_bits(f.bits)
            };
            set_word(&mut to, f.offset, v);
        }
        to
    }

    /// `MSG_WriteDeltaPlayerstate` (`from` None: against a zeroed state).
    pub fn write_delta_playerstate(&mut self, from: Option<&PlayerState>, to: &PlayerState) {
        let dummy = PlayerState::default();
        let from = from.unwrap_or(&dummy);
        let mut lc = 0;
        for (i, f) in PLAYER_STATE_FIELDS.iter().enumerate() {
            if word(from, f.offset) != word(to, f.offset) {
                lc = i + 1;
            }
        }
        self.write_byte(lc as i32); // # of changes
        for f in &PLAYER_STATE_FIELDS[..lc] {
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
        let mut statsbits = 0;
        for i in 0..MAX_STATS {
            if to.stats[i] != from.stats[i] {
                statsbits |= 1 << i;
            }
        }
        let mut persistantbits = 0;
        for i in 0..MAX_PERSISTANT {
            if to.persistant[i] != from.persistant[i] {
                persistantbits |= 1 << i;
            }
        }
        let mut miscbits = 0;
        for i in 0..MAX_MISC {
            if to.misc[i] != from.misc[i] {
                miscbits |= 1 << i;
            }
        }
        if statsbits == 0 && persistantbits == 0 && miscbits == 0 {
            self.write_bits(0, 1); // no change
            return;
        }
        self.write_bits(1, 1); // changed
        if statsbits != 0 {
            self.write_bits(1, 1); // changed
            self.write_bits(statsbits, MAX_STATS as i32);
            for i in 0..MAX_STATS {
                if statsbits & (1 << i) != 0 {
                    self.write_short(to.stats[i]);
                }
            }
        } else {
            self.write_bits(0, 1); // no change
        }
        if persistantbits != 0 {
            self.write_bits(1, 1); // changed
            self.write_bits(persistantbits, MAX_PERSISTANT as i32);
            for i in 0..MAX_PERSISTANT {
                if persistantbits & (1 << i) != 0 {
                    self.write_short(to.persistant[i]);
                }
            }
        } else {
            self.write_bits(0, 1); // no change
        }
        if miscbits != 0 {
            self.write_bits(1, 1); // changed
            self.write_bits(miscbits, MAX_MISC as i32);
            for i in 0..MAX_MISC {
                if miscbits & (1 << i) != 0 {
                    self.write_long(to.misc[i]);
                }
            }
        } else {
            self.write_bits(0, 1); // no change
        }
    }

    /// `MSG_ReadDeltaPlayerstate`.
    pub fn read_delta_playerstate(&mut self, from: Option<&PlayerState>) -> PlayerState {
        let dummy = PlayerState::default();
        let from = from.unwrap_or(&dummy);
        let mut to = *from;
        let mut lc = self.read_byte();
        if lc > PLAYER_STATE_FIELDS.len() as i32 || lc < 0 {
            // ERR_DROP "invalid playerState field count"
            lc = PLAYER_STATE_FIELDS.len() as i32;
        }
        for f in &PLAYER_STATE_FIELDS[..lc as usize] {
            if self.read_bits(1) == 0 {
                // no change
                continue;
            }
            let v = if f.bits == 0 { self.read_net_float() } else { self.read_bits(f.bits) };
            set_word(&mut to, f.offset, v);
        }
        // read the arrays
        if self.read_bits(1) != 0 {
            // parse stats
            if self.read_bits(1) != 0 {
                let bits = self.read_bits(MAX_STATS as i32);
                for i in 0..MAX_STATS {
                    if bits & (1 << i) != 0 {
                        to.stats[i] = self.read_short();
                    }
                }
            }
            // parse persistant stats
            if self.read_bits(1) != 0 {
                let bits = self.read_bits(MAX_PERSISTANT as i32);
                for i in 0..MAX_PERSISTANT {
                    if bits & (1 << i) != 0 {
                        to.persistant[i] = self.read_short();
                    }
                }
            }
            // parse misc data
            if self.read_bits(1) != 0 {
                let bits = self.read_bits(MAX_MISC as i32);
                for i in 0..MAX_MISC {
                    if bits & (1 << i) != 0 {
                        to.misc[i] = self.read_long();
                    }
                }
            }
        }
        to
    }
}
