//! Kernel `md5` — qcommon/md5.c: Colin Plumb's public-domain MD5
//! (`MD5Init` / `MD5Update` / `MD5Final` / `MD5Transform`) and
//! `Com_MD5File`, which the client uses for cl_guid (`CL_UpdateGUID`: the
//! MD5 of the 2048-byte qkey file, prefixed with the server address when
//! cl_guidServerUniq is set) (dec033).

/// `struct MD5Context`.
#[derive(Clone, Debug)]
pub struct Md5 {
    buf: [u32; 4],
    bits: [u32; 2],
    input: [u8; 64],
}

impl Default for Md5 {
    /// `MD5Init`.
    fn default() -> Self {
        Md5 { buf: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476], bits: [0, 0], input: [0; 64] }
    }
}

macro_rules! step {
    ($f:expr, $w:expr, $x:expr, $data:expr, $s:expr) => {
        $w = $w.wrapping_add($f).wrapping_add($data);
        $w = $w.rotate_left($s);
        $w = $w.wrapping_add($x);
    };
}

/// `MD5Transform`: the core of the algorithm.
fn transform(buf: &mut [u32; 4], block: &[u8; 64]) {
    let mut x = [0u32; 16];
    for (i, v) in x.iter_mut().enumerate() {
        *v = u32::from_le_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
    }
    let (mut a, mut b, mut c, mut d) = (buf[0], buf[1], buf[2], buf[3]);
    let f1 = |x: u32, y: u32, z: u32| z ^ (x & (y ^ z));
    let f2 = |x: u32, y: u32, z: u32| f1(z, x, y);
    let f3 = |x: u32, y: u32, z: u32| x ^ y ^ z;
    let f4 = |x: u32, y: u32, z: u32| y ^ (x | !z);
    const K1: [(usize, u32, u32); 16] = [
        (0, 0xd76aa478, 7), (1, 0xe8c7b756, 12), (2, 0x242070db, 17), (3, 0xc1bdceee, 22),
        (4, 0xf57c0faf, 7), (5, 0x4787c62a, 12), (6, 0xa8304613, 17), (7, 0xfd469501, 22),
        (8, 0x698098d8, 7), (9, 0x8b44f7af, 12), (10, 0xffff5bb1, 17), (11, 0x895cd7be, 22),
        (12, 0x6b901122, 7), (13, 0xfd987193, 12), (14, 0xa679438e, 17), (15, 0x49b40821, 22),
    ];
    const K2: [(usize, u32, u32); 16] = [
        (1, 0xf61e2562, 5), (6, 0xc040b340, 9), (11, 0x265e5a51, 14), (0, 0xe9b6c7aa, 20),
        (5, 0xd62f105d, 5), (10, 0x02441453, 9), (15, 0xd8a1e681, 14), (4, 0xe7d3fbc8, 20),
        (9, 0x21e1cde6, 5), (14, 0xc33707d6, 9), (3, 0xf4d50d87, 14), (8, 0x455a14ed, 20),
        (13, 0xa9e3e905, 5), (2, 0xfcefa3f8, 9), (7, 0x676f02d9, 14), (12, 0x8d2a4c8a, 20),
    ];
    const K3: [(usize, u32, u32); 16] = [
        (5, 0xfffa3942, 4), (8, 0x8771f681, 11), (11, 0x6d9d6122, 16), (14, 0xfde5380c, 23),
        (1, 0xa4beea44, 4), (4, 0x4bdecfa9, 11), (7, 0xf6bb4b60, 16), (10, 0xbebfbc70, 23),
        (13, 0x289b7ec6, 4), (0, 0xeaa127fa, 11), (3, 0xd4ef3085, 16), (6, 0x04881d05, 23),
        (9, 0xd9d4d039, 4), (12, 0xe6db99e5, 11), (15, 0x1fa27cf8, 16), (2, 0xc4ac5665, 23),
    ];
    const K4: [(usize, u32, u32); 16] = [
        (0, 0xf4292244, 6), (7, 0x432aff97, 10), (14, 0xab9423a7, 15), (5, 0xfc93a039, 21),
        (12, 0x655b59c3, 6), (3, 0x8f0ccc92, 10), (10, 0xffeff47d, 15), (1, 0x85845dd1, 21),
        (8, 0x6fa87e4f, 6), (15, 0xfe2ce6e0, 10), (6, 0xa3014314, 15), (13, 0x4e0811a1, 21),
        (4, 0xf7537e82, 6), (11, 0xbd3af235, 10), (2, 0x2ad7d2bb, 15), (9, 0xeb86d391, 21),
    ];
    for (round, table) in [K1, K2, K3, K4].iter().enumerate() {
        for (i, &(k, t, s)) in table.iter().enumerate() {
            let f = |x: u32, y: u32, z: u32| match round {
                0 => f1(x, y, z),
                1 => f2(x, y, z),
                2 => f3(x, y, z),
                _ => f4(x, y, z),
            };
            // the C's MD5STEP rotates the roles of a, b, c, d each step
            match i % 4 {
                0 => {
                    step!(f(b, c, d), a, b, x[k].wrapping_add(t), s);
                }
                1 => {
                    step!(f(a, b, c), d, a, x[k].wrapping_add(t), s);
                }
                2 => {
                    step!(f(d, a, b), c, d, x[k].wrapping_add(t), s);
                }
                _ => {
                    step!(f(c, d, a), b, c, x[k].wrapping_add(t), s);
                }
            }
        }
    }
    buf[0] = buf[0].wrapping_add(a);
    buf[1] = buf[1].wrapping_add(b);
    buf[2] = buf[2].wrapping_add(c);
    buf[3] = buf[3].wrapping_add(d);
}

impl Md5 {
    /// `MD5Update`.
    pub fn update(&mut self, mut data: &[u8]) {
        let len = data.len() as u32;
        // update bitcount
        let t = self.bits[0];
        self.bits[0] = t.wrapping_add(len << 3);
        if self.bits[0] < t {
            self.bits[1] = self.bits[1].wrapping_add(1); // carry from low to high
        }
        self.bits[1] = self.bits[1].wrapping_add(len >> 29);
        let t = ((t >> 3) & 0x3f) as usize; // bytes already in shsInfo->data
        // handle any leading odd-sized chunks
        if t != 0 {
            let room = 64 - t;
            if data.len() < room {
                self.input[t..t + data.len()].copy_from_slice(data);
                return;
            }
            self.input[t..].copy_from_slice(&data[..room]);
            let block = self.input;
            transform(&mut self.buf, &block);
            data = &data[room..];
        }
        // process data in 64-byte chunks
        while data.len() >= 64 {
            self.input.copy_from_slice(&data[..64]);
            let block = self.input;
            transform(&mut self.buf, &block);
            data = &data[64..];
        }
        // handle any remaining bytes of data
        self.input[..data.len()].copy_from_slice(data);
    }

    /// `MD5Final`: pad to 56 mod 64, append the length, the digest.
    pub fn finalize(mut self) -> [u8; 16] {
        let count = ((self.bits[0] >> 3) & 0x3f) as usize;
        let mut p = count;
        // set the first char of padding to 0x80; this is safe since there
        // is always at least one byte free
        self.input[p] = 0x80;
        p += 1;
        let left = 64 - 1 - count;
        if left < 8 {
            // two lots of padding: pad the first block to 64 bytes
            self.input[p..].fill(0);
            let block = self.input;
            transform(&mut self.buf, &block);
            // now fill the next block with 56 bytes
            self.input[..56].fill(0);
        } else {
            // pad block to 56 bytes
            self.input[p..56].fill(0);
        }
        // append length in bits and transform
        self.input[56..60].copy_from_slice(&self.bits[0].to_le_bytes());
        self.input[60..64].copy_from_slice(&self.bits[1].to_le_bytes());
        let block = self.input;
        transform(&mut self.buf, &block);
        let mut digest = [0u8; 16];
        for (i, v) in self.buf.iter().enumerate() {
            digest[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        digest
    }
}

/// `Com_MD5File` over bytes already read: the first `length` bytes of
/// `data` (all when 0 or longer), after `prefix`; upper-case hex ("" for
/// an empty file).
pub fn md5_file(data: &[u8], length: usize, prefix: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }
    let n = if length == 0 || data.len() < length { data.len() } else { length };
    let mut m = Md5::default();
    if !prefix.is_empty() {
        m.update(prefix);
    }
    m.update(&data[..n]);
    m.finalize().iter().map(|b| format!("{b:02X}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc1321_vectors() {
        let hex = |s: &str| {
            let mut m = Md5::default();
            m.update(s.as_bytes());
            m.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()
        };
        assert_eq!(hex(""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex("message digest"), "f96b697d7cb7938d525a2f31aaf161d0");
        assert_eq!(hex("12345678901234567890123456789012345678901234567890123456789012345678901234567890"), "57edf4a22be3c955ac49da2e2107b67a");
        assert_eq!(md5_file(b"bc", 0, b"a"), "900150983CD24FB0D6963F7D28E17F72");
    }
}
