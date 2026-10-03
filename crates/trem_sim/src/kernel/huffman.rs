//! Kernel `huffman` — qcommon/huffman.c: the adaptive Huffman coder of the
//! network layer (Sayood's algorithm; ranks are implicit in a doubly-linked
//! list). Two uses, as in the original:
//!
//! * `Huff_Compress` / `Huff_Decompress`: a fresh adaptive tree per message
//!   (the connectionless `connect` packet, compressed from byte 12 on);
//! * `MSG_initHuffman`: the fixed tree every netchan bitstream uses, built
//!   once by feeding `msg_hData` (msg.c) symbol counts to `Huff_addRef`, then
//!   read by `Huff_offsetTransmit` / `Huff_offsetReceive` (msg.rs).
//!
//! The tree lives in an index arena (`nodeList[768]`); the `node_t **head`
//! block pointers are cells of `nodePtrs`, recycled through a free list
//! exactly like `get_ppnode` / `free_ppnode`.

use std::sync::OnceLock;

/// `HMAX`: maximum symbol.
pub const HMAX: usize = 256;
/// `NYT`: not yet transmitted.
pub const NYT: i32 = HMAX as i32;
/// `INTERNAL_NODE`.
pub const INTERNAL_NODE: i32 = HMAX as i32 + 1;
const NIL: usize = usize::MAX;

#[derive(Clone, Copy, Debug)]
struct Node {
    left: usize,
    right: usize,
    parent: usize,
    next: usize,
    prev: usize,
    /// index into `ptrs` (the `node_t **head` cell)
    head: usize,
    weight: i32,
    symbol: i32,
}

const EMPTY: Node = Node { left: NIL, right: NIL, parent: NIL, next: NIL, prev: NIL, head: NIL, weight: 0, symbol: 0 };

/// `huff_t`.
#[derive(Clone, Debug)]
pub struct Huff {
    tree: usize,
    lhead: usize,
    loc: [usize; HMAX + 1],
    nodes: Vec<Node>,
    /// `nodePtrs`: each cell holds a node index
    ptrs: Vec<usize>,
    /// `freelist` of `nodePtrs` cells (LIFO, like the C's linked cells)
    freelist: Vec<usize>,
}

impl Default for Huff {
    fn default() -> Self {
        Self::new()
    }
}

impl Huff {
    /// `Huff_Init` for one coder: the tree and list hold only the NYT node.
    pub fn new() -> Self {
        let mut h = Huff { tree: 0, lhead: 0, loc: [NIL; HMAX + 1], nodes: Vec::with_capacity(768), ptrs: Vec::with_capacity(768), freelist: Vec::new() };
        h.nodes.push(Node { symbol: NYT, weight: 0, ..EMPTY });
        h.tree = 0;
        h.lhead = 0;
        h.loc[NYT as usize] = 0;
        h
    }

    /// `get_ppnode`.
    fn get_ppnode(&mut self) -> usize {
        if let Some(c) = self.freelist.pop() {
            c
        } else {
            self.ptrs.push(NIL);
            self.ptrs.len() - 1
        }
    }

    /// `free_ppnode`.
    fn free_ppnode(&mut self, cell: usize) {
        self.freelist.push(cell);
    }

    /// `swap`: exchange two nodes' places in the tree.
    fn swap(&mut self, node1: usize, node2: usize) {
        let par1 = self.nodes[node1].parent;
        let par2 = self.nodes[node2].parent;
        if par1 != NIL {
            if self.nodes[par1].left == node1 {
                self.nodes[par1].left = node2;
            } else {
                self.nodes[par1].right = node2;
            }
        } else {
            self.tree = node2;
        }
        if par2 != NIL {
            if self.nodes[par2].left == node2 {
                self.nodes[par2].left = node1;
            } else {
                self.nodes[par2].right = node1;
            }
        } else {
            self.tree = node1;
        }
        self.nodes[node1].parent = par2;
        self.nodes[node2].parent = par1;
    }

    /// `swaplist`: exchange two nodes in the rank list.
    fn swaplist(&mut self, node1: usize, node2: usize) {
        let par1 = self.nodes[node1].next;
        self.nodes[node1].next = self.nodes[node2].next;
        self.nodes[node2].next = par1;
        let par1 = self.nodes[node1].prev;
        self.nodes[node1].prev = self.nodes[node2].prev;
        self.nodes[node2].prev = par1;
        if self.nodes[node1].next == node1 {
            self.nodes[node1].next = node2;
        }
        if self.nodes[node2].next == node2 {
            self.nodes[node2].next = node1;
        }
        for n in [node1, node2] {
            let nx = self.nodes[n].next;
            if nx != NIL {
                self.nodes[nx].prev = n;
            }
            let pv = self.nodes[n].prev;
            if pv != NIL {
                self.nodes[pv].next = n;
            }
        }
    }

    /// `increment` (iterative over the parents, same order as the recursion).
    fn increment(&mut self, mut node: usize) {
        // the C recurses on node->parent before the parent fix-up below,
        // so keep the pending fix-ups on a stack
        let mut stack = Vec::new();
        while node != NIL {
            let nx = self.nodes[node].next;
            if nx != NIL && self.nodes[nx].weight == self.nodes[node].weight {
                let lnode = self.ptrs[self.nodes[node].head];
                if lnode != self.nodes[node].parent {
                    self.swap(lnode, node);
                }
                self.swaplist(lnode, node);
            }
            let pv = self.nodes[node].prev;
            if pv != NIL && self.nodes[pv].weight == self.nodes[node].weight {
                let h = self.nodes[node].head;
                self.ptrs[h] = pv;
            } else {
                let h = self.nodes[node].head;
                self.ptrs[h] = NIL;
                self.free_ppnode(h);
            }
            self.nodes[node].weight += 1;
            let nx = self.nodes[node].next;
            if nx != NIL && self.nodes[nx].weight == self.nodes[node].weight {
                self.nodes[node].head = self.nodes[nx].head;
            } else {
                let c = self.get_ppnode();
                self.nodes[node].head = c;
                self.ptrs[c] = node;
            }
            let parent = self.nodes[node].parent;
            if parent != NIL {
                stack.push(node);
            }
            node = parent;
        }
        while let Some(node) = stack.pop() {
            let parent = self.nodes[node].parent;
            if self.nodes[node].prev == parent {
                self.swaplist(node, parent);
                let h = self.nodes[node].head;
                if self.ptrs[h] == node {
                    self.ptrs[h] = parent;
                }
            }
        }
    }

    /// `Huff_addRef`.
    pub fn add_ref(&mut self, ch: u8) {
        let ch = ch as usize;
        if self.loc[ch] != NIL {
            let n = self.loc[ch];
            self.increment(n);
            return;
        }
        // first transmission of this symbol
        let tnode = self.nodes.len();
        self.nodes.push(EMPTY);
        let tnode2 = self.nodes.len();
        self.nodes.push(EMPTY);
        let lhead = self.lhead;
        {
            self.nodes[tnode2].symbol = INTERNAL_NODE;
            self.nodes[tnode2].weight = 1;
            let lnext = self.nodes[lhead].next;
            self.nodes[tnode2].next = lnext;
            if lnext != NIL {
                self.nodes[lnext].prev = tnode2;
                if self.nodes[lnext].weight == 1 {
                    self.nodes[tnode2].head = self.nodes[lnext].head;
                } else {
                    let c = self.get_ppnode();
                    self.nodes[tnode2].head = c;
                    self.ptrs[c] = tnode2;
                }
            } else {
                let c = self.get_ppnode();
                self.nodes[tnode2].head = c;
                self.ptrs[c] = tnode2;
            }
            self.nodes[lhead].next = tnode2;
            self.nodes[tnode2].prev = lhead;
        }
        {
            self.nodes[tnode].symbol = ch as i32;
            self.nodes[tnode].weight = 1;
            let lnext = self.nodes[lhead].next;
            self.nodes[tnode].next = lnext;
            if lnext != NIL {
                self.nodes[lnext].prev = tnode;
                if self.nodes[lnext].weight == 1 {
                    self.nodes[tnode].head = self.nodes[lnext].head;
                } else {
                    // this should never happen
                    let c = self.get_ppnode();
                    self.nodes[tnode].head = c;
                    self.ptrs[c] = tnode2;
                }
            } else {
                // this should never happen
                let c = self.get_ppnode();
                self.nodes[tnode].head = c;
                self.ptrs[c] = tnode;
            }
            self.nodes[lhead].next = tnode;
            self.nodes[tnode].prev = lhead;
            self.nodes[tnode].left = NIL;
            self.nodes[tnode].right = NIL;
        }
        let lparent = self.nodes[lhead].parent;
        if lparent != NIL {
            // lhead is guaranteed to be the NYT
            if self.nodes[lparent].left == lhead {
                self.nodes[lparent].left = tnode2;
            } else {
                self.nodes[lparent].right = tnode2;
            }
        } else {
            self.tree = tnode2;
        }
        self.nodes[tnode2].right = tnode;
        self.nodes[tnode2].left = lhead;
        self.nodes[tnode2].parent = lparent;
        self.nodes[lhead].parent = tnode2;
        self.nodes[tnode].parent = tnode2;
        self.loc[ch] = tnode;
        let p = self.nodes[tnode2].parent;
        self.increment(p);
    }

    /// `send`: the prefix code of `node`, root first.
    fn send(&self, node: usize, out: &mut Vec<u8>) {
        let mut path = Vec::new();
        let mut n = node;
        while self.nodes[n].parent != NIL {
            let p = self.nodes[n].parent;
            path.push(if self.nodes[p].right == n { 1u8 } else { 0u8 });
            n = p;
        }
        out.extend(path.iter().rev());
    }

    /// The code bits of `ch` (`send( huff->loc[ch], NULL, ... )`); empty if
    /// the symbol was never added.
    pub fn code(&self, ch: i32) -> Vec<u8> {
        let mut v = Vec::new();
        let n = self.loc[ch as usize];
        if n != NIL {
            self.send(n, &mut v);
        }
        v
    }

    /// `Huff_transmit` (adaptive: NYT + 8 raw bits for a new symbol).
    fn transmit(&self, ch: i32, out: &mut BitWriter) {
        if self.loc[ch as usize] == NIL {
            self.transmit(NYT, out);
            for i in (0..8).rev() {
                out.add_bit(((ch >> i) & 1) as u8);
            }
        } else {
            let mut v = Vec::new();
            self.send(self.loc[ch as usize], &mut v);
            for b in v {
                out.add_bit(b);
            }
        }
    }

    /// `Huff_Receive` / `Huff_offsetReceive`: walk from the root, one bit per
    /// internal node; 0 for an illegal tree (the C's `*ch = 0`).
    pub fn receive(&self, get_bit: &mut dyn FnMut() -> u8) -> i32 {
        let mut node = self.tree;
        while node != NIL && self.nodes[node].symbol == INTERNAL_NODE {
            node = if get_bit() != 0 { self.nodes[node].right } else { self.nodes[node].left };
        }
        if node == NIL {
            return 0;
        }
        self.nodes[node].symbol
    }
}

/// The bloc/offset bit cursor of huffman.c (`add_bit` / `get_bit`).
pub struct BitWriter<'a> {
    pub buf: &'a mut [u8],
    pub bloc: usize,
}

impl BitWriter<'_> {
    /// `add_bit` / `Huff_putBit`: a byte is cleared when its first bit is written.
    #[inline]
    pub fn add_bit(&mut self, bit: u8) {
        let i = self.bloc >> 3;
        if i < self.buf.len() {
            if self.bloc & 7 == 0 {
                self.buf[i] = 0;
            }
            self.buf[i] |= bit << (self.bloc & 7);
        }
        self.bloc += 1;
    }
}

/// `get_bit` / `Huff_getBit` over a byte buffer (0 past its end).
#[inline]
pub fn get_bit(buf: &[u8], bloc: &mut usize) -> u8 {
    let t = buf.get(*bloc >> 3).map(|b| (b >> (*bloc & 7)) & 1).unwrap_or(0);
    *bloc += 1;
    t
}

/// `Huff_Compress( mbuf, offset )`: `data[offset..]` becomes a 2-byte
/// big-endian length followed by the adaptive code.
pub fn compress(data: &mut Vec<u8>, offset: usize) {
    if data.len() <= offset {
        return;
    }
    let size = data.len() - offset;
    let mut seq = vec![0u8; 65536];
    seq[0] = (size >> 8) as u8;
    seq[1] = (size & 0xff) as u8;
    let mut huff = Huff::new();
    let mut w = BitWriter { buf: &mut seq, bloc: 16 };
    for i in 0..size {
        let ch = data[offset + i];
        huff.transmit(ch as i32, &mut w);
        huff.add_ref(ch);
    }
    let bloc = w.bloc + 8; // next byte
    let n = bloc >> 3;
    data.truncate(offset);
    data.extend_from_slice(&seq[..n]);
}

/// `Huff_Decompress( mbuf, offset )` with `maxsize` the message buffer's
/// capacity (a bad length is clamped to it, as in the C).
pub fn decompress(data: &mut Vec<u8>, offset: usize, maxsize: usize) {
    if data.len() <= offset {
        return;
    }
    let size = data.len() - offset;
    let buffer: Vec<u8> = data[offset..].to_vec();
    let mut cch = buffer[0] as usize * 256 + buffer.get(1).copied().unwrap_or(0) as usize;
    // don't overflow with bad messages
    if cch > maxsize.saturating_sub(offset) {
        cch = maxsize.saturating_sub(offset);
    }
    let mut huff = Huff::new();
    let mut bloc = 16usize;
    let mut seq = Vec::with_capacity(cch);
    for _ in 0..cch {
        // don't overflow reading from the messages
        if (bloc >> 3) > size {
            seq.push(0);
            break;
        }
        let mut ch = huff.receive(&mut || get_bit(&buffer, &mut bloc));
        if ch == NYT {
            ch = 0;
            for _ in 0..8 {
                ch = (ch << 1) + get_bit(&buffer, &mut bloc) as i32;
            }
        }
        seq.push(ch as u8);
        huff.add_ref(ch as u8);
    }
    // the C copies cch bytes (a short read leaves the rest as they were)
    seq.resize(cch, 0);
    data.truncate(offset);
    data.extend_from_slice(&seq);
}

/// `msg_hData`: symbol frequencies that seed the netchan Huffman tree (msg.c).
pub const MSG_HDATA: [i32; 256] = [
    250315, 41193, 6292, 7106, 3730, 3750, 6110, 23283, 33317, 6950, 7838, 9714, 9257, 17259, 3949, 1778, // 0
    8288, 1604, 1590, 1663, 1100, 1213, 1238, 1134, 1749, 1059, 1246, 1149, 1273, 4486, 2805, 3472, // 16
    21819, 1159, 1670, 1066, 1043, 1012, 1053, 1070, 1726, 888, 1180, 850, 960, 780, 1752, 3296, // 32
    10630, 4514, 5881, 2685, 4650, 3837, 2093, 1867, 2584, 1949, 1972, 940, 1134, 1788, 1670, 1206, // 48
    5719, 6128, 7222, 6654, 3710, 3795, 1492, 1524, 2215, 1140, 1355, 971, 2180, 1248, 1328, 1195, // 64
    1770, 1078, 1264, 1266, 1168, 965, 1155, 1186, 1347, 1228, 1529, 1600, 2617, 2048, 2546, 3275, // 80
    2410, 3585, 2504, 2800, 2675, 6146, 3663, 2840, 14253, 3164, 2221, 1687, 3208, 2739, 3512, 4796, // 96
    4091, 3515, 5288, 4016, 7937, 6031, 5360, 3924, 4892, 3743, 4566, 4807, 5852, 6400, 6225, 8291, // 112
    23243, 7838, 7073, 8935, 5437, 4483, 3641, 5256, 5312, 5328, 5370, 3492, 2458, 1694, 1821, 2121, // 128
    1916, 1149, 1516, 1367, 1236, 1029, 1258, 1104, 1245, 1006, 1149, 1025, 1241, 952, 1287, 997, // 144
    1713, 1009, 1187, 879, 1099, 929, 1078, 951, 1656, 930, 1153, 1030, 1262, 1062, 1214, 1060, // 160
    1621, 930, 1106, 912, 1034, 892, 1158, 990, 1175, 850, 1121, 903, 1087, 920, 1144, 1056, // 176
    3462, 2240, 4397, 12136, 7758, 1345, 1307, 3278, 1950, 886, 1023, 1112, 1077, 1042, 1061, 1071, // 192
    1484, 1001, 1096, 915, 1052, 995, 1070, 876, 1111, 851, 1059, 805, 1112, 923, 1103, 817, // 208
    1899, 1872, 976, 841, 1127, 956, 1159, 950, 7791, 954, 1289, 933, 1127, 3207, 1020, 927, // 224
    1355, 768, 1040, 745, 952, 805, 1073, 740, 1013, 805, 1008, 796, 996, 1057, 11457, 13504, // 240
];

/// The netchan coder (`msgHuff` after `MSG_initHuffman`): the compressor
/// and decompressor are built by the same `Huff_addRef` sequence, so one
/// tree serves both; each symbol's code is cached for `Huff_offsetTransmit`.
pub struct MsgHuff {
    pub tree: Huff,
    pub codes: Vec<Vec<u8>>,
}

/// `MSG_initHuffman` (run once).
pub fn msg_huff() -> &'static MsgHuff {
    static H: OnceLock<MsgHuff> = OnceLock::new();
    H.get_or_init(|| {
        let mut h = Huff::new();
        for (i, &n) in MSG_HDATA.iter().enumerate() {
            for _ in 0..n {
                h.add_ref(i as u8);
            }
        }
        let codes = (0..256).map(|c| h.code(c)).collect();
        MsgHuff { tree: h, codes }
    })
}
