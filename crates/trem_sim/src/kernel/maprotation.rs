//! Kernel `maprotation` — game/g_maprotation.c: maprotation.cfg parsing and
//! the rotation state machine (maps, labels, goto/resume, return, nested
//! rotations, `if numClients|lastWin|random` conditions).
//!
//! In the original the state lives in cvars (g_currentMapRotation,
//! g_mapRotationNodes, g_mapRotationStack) so it survives map changes; here
//! it is a plain struct the host keeps across World reloads. Advancing yields
//! the map to load and its post-map command section (dec018 superseded).
//!
//! Where the C helpers went: `G_MapExists` / `G_RotationExists` are the
//! validation pass in `parse`; `G_AllocateNode` / `G_FreeNode` are Vec pushes;
//! `G_PushRotationStack` / `G_PopRotationStack` are the `stack` Vec;
//! `G_RotationNameByIndex`, `G_CurrentNodeIndexArray`, `G_CurrentNodeIndex`,
//! `G_NodeByIndex` and `G_SetCurrentNodeByIndex` index `current` / `nodes`
//! directly; `G_NodeIndexAfter` is `node_after`; `G_IssueMapChange` fills
//! `pending`; `G_InitMapRotations` / `G_ShutdownMapRotations` are the host's
//! (trem_godot game.rs init_map_rotations, and dropping the struct);
//! `G_PrintRotations` / `G_PrintSpaces` are `log`; `Svcmd_MapRotation_f` /
//! `Svcmd_G_AdvanceMapRotation_f` are the console's `maprotation` command and
//! `advance`.

use crate::gen::enums::Team;

const MAX_MAP_ROTATIONS: usize = 64;
const MAX_MAP_ROTATION_MAPS: usize = 256;
/// `NOT_ROTATING`.
pub const NOT_ROTATING: i32 = -1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Var {
    Random,
    NumClients,
    LastWin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Lt,
    Eq,
    Gt,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Map { name: String, post_command: String, layouts: String },
    Condition { lhs: Var, op: Op, num_clients: i32, last_win: Team, target: Box<Node> },
    Goto(String),
    Resume(String),
    Label(String),
    Return,
}

#[derive(Clone, Debug, Default)]
pub struct Rotation {
    pub name: String,
    pub nodes: Vec<Node>,
}

/// What a step decided: the map to load and the console commands to run after it.
#[derive(Clone, Debug, PartialEq)]
pub struct MapChange {
    pub map: String,
    pub post_command: String,
    pub layouts: String,
}

/// The facts conditions read (level.numConnectedClients, level.lastWin) and
/// G_MapExists / rand().
pub struct Ctx<'a> {
    pub num_connected: i32,
    pub last_win: Option<Team>,
    pub map_exists: &'a dyn Fn(&str) -> bool,
    pub rand: &'a mut dyn FnMut() -> i32,
}

#[derive(Clone, Debug)]
pub struct MapRotations {
    pub rotations: Vec<Rotation>,
    /// g_currentMapRotation
    pub current: i32,
    /// g_mapRotationNodes
    pub nodes: Vec<usize>,
    /// g_mapRotationStack (top first)
    pub stack: Vec<i32>,
    /// G_Printf output (warnings / errors)
    pub log: Vec<String>,
    pending: Option<MapChange>,
}

impl Default for MapRotations {
    fn default() -> Self {
        MapRotations { rotations: Vec::new(), current: NOT_ROTATING, nodes: Vec::new(), stack: Vec::new(), log: Vec::new(), pending: None }
    }
}

// ------------------------------------------------------------- tokenizer
/// COM_Parse / COM_ParseExt over a byte cursor.
pub(crate) struct Lexer<'a> {
    s: &'a [u8],
    p: usize,
}

impl<'a> Lexer<'a> {
    pub(crate) fn new(s: &'a str) -> Self {
        Lexer { s: s.as_bytes(), p: 0 }
    }
    /// `COM_ParseExt(allowLineBreaks)`; "" at end (or at a line break when
    /// line breaks aren't allowed).
    pub(crate) fn parse_ext(&mut self, allow_line_breaks: bool) -> String {
        let s = self.s;
        let mut has_newlines = false;
        loop {
            // skip whitespace
            while self.p < s.len() && s[self.p] <= b' ' {
                if s[self.p] == b'\n' {
                    has_newlines = true;
                }
                self.p += 1;
            }
            if self.p >= s.len() {
                return String::new();
            }
            if has_newlines && !allow_line_breaks {
                return String::new();
            }
            if s[self.p] == b'/' && s.get(self.p + 1) == Some(&b'/') {
                while self.p < s.len() && s[self.p] != b'\n' {
                    self.p += 1;
                }
            } else if s[self.p] == b'/' && s.get(self.p + 1) == Some(&b'*') {
                self.p += 2;
                while self.p < s.len() && !(s[self.p] == b'*' && s.get(self.p + 1) == Some(&b'/')) {
                    self.p += 1;
                }
                self.p = (self.p + 2).min(s.len());
            } else {
                break;
            }
        }
        let mut out = Vec::new();
        if s[self.p] == b'"' {
            self.p += 1;
            while self.p < s.len() && s[self.p] != b'"' {
                out.push(s[self.p]);
                self.p += 1;
            }
            self.p = (self.p + 1).min(s.len());
        } else {
            while self.p < s.len() && s[self.p] > b' ' {
                out.push(s[self.p]);
                self.p += 1;
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }
    pub(crate) fn parse(&mut self) -> String {
        self.parse_ext(true)
    }
}

fn atoi(s: &str) -> i32 {
    crate::kernel::spawn::c_atoi(s)
}

// ---------------------------------------------------------------- parsing
/// `G_ParseMapCommandSection`.
fn parse_command_section(lx: &mut Lexer, post: &mut String, layouts: &mut String) -> bool {
    loop {
        let tok = lx.parse();
        if tok.is_empty() {
            return false;
        }
        if tok == "}" {
            if post.ends_with(';') {
                post.pop();
                post.push('\n');
            }
            return true;
        }
        if tok.eq_ignore_ascii_case("layouts") {
            layouts.clear();
            loop {
                let t = lx.parse_ext(false);
                if t.is_empty() {
                    break;
                }
                layouts.push_str(&t);
                layouts.push(' ');
            }
            continue;
        }
        post.push_str(&tok);
        post.push(' ');
        loop {
            let t = lx.parse_ext(false);
            if t.is_empty() {
                break;
            }
            post.push_str(&t);
            post.push(' ');
        }
        post.pop();
        post.push(';');
    }
}

/// `G_ParseNode`.
fn parse_node(lx: &mut Lexer, tok: &str, conditional: bool, log: &mut Vec<String>) -> Option<Node> {
    if tok.eq_ignore_ascii_case("if") {
        let t = lx.parse();
        if t.is_empty() {
            return None;
        }
        let lhs;
        let (mut op, mut num, mut win) = (Op::Eq, 0, Team::None);
        if t.eq_ignore_ascii_case("numClients") {
            lhs = Var::NumClients;
            let o = lx.parse();
            op = match o.as_str() {
                "<" => Op::Lt,
                ">" => Op::Gt,
                "=" => Op::Eq,
                "" => return None,
                _ => {
                    log.push(format!("ERROR: invalid operator in expression: {o}"));
                    return None;
                }
            };
            let n = lx.parse();
            if n.is_empty() {
                return None;
            }
            num = atoi(&n);
        } else if t.eq_ignore_ascii_case("lastWin") {
            lhs = Var::LastWin;
            let r = lx.parse();
            win = if r.eq_ignore_ascii_case("aliens") {
                Team::Aliens
            } else if r.eq_ignore_ascii_case("humans") {
                Team::Humans
            } else {
                if !r.is_empty() {
                    log.push(format!("ERROR: invalid right hand side in expression: {r}"));
                }
                return None;
            };
        } else if t.eq_ignore_ascii_case("random") {
            lhs = Var::Random;
        } else {
            log.push(format!("ERROR: invalid left hand side in expression: {t}"));
            return None;
        }
        let next = lx.parse();
        if next.is_empty() {
            return None;
        }
        let target = parse_node(lx, &next, true, log)?;
        return Some(Node::Condition { lhs, op, num_clients: num, last_win: win, target: Box::new(target) });
    }
    if tok.eq_ignore_ascii_case("return") {
        return Some(Node::Return);
    }
    if tok.eq_ignore_ascii_case("goto") || tok.eq_ignore_ascii_case("resume") {
        let l = lx.parse();
        if l.is_empty() {
            log.push("ERROR: goto or resume without label".into());
            return None;
        }
        return Some(if tok.eq_ignore_ascii_case("goto") { Node::Goto(l) } else { Node::Resume(l) });
    }
    if tok.starts_with('#') || conditional {
        return Some(if conditional { Node::Goto(tok.to_string()) } else { Node::Label(tok.to_string()) });
    }
    Some(Node::Map { name: tok.to_string(), post_command: String::new(), layouts: String::new() })
}

/// `G_ParseMapRotation`.
fn parse_rotation(lx: &mut Lexer, mr: &mut Rotation, log: &mut Vec<String>) -> bool {
    loop {
        let tok = lx.parse();
        if tok.is_empty() {
            return false;
        }
        if tok == "{" {
            match mr.nodes.last_mut() {
                Some(Node::Map { post_command, layouts, .. }) => {
                    if !parse_command_section(lx, post_command, layouts) {
                        log.push("ERROR: failed to parse map command section".into());
                        return false;
                    }
                }
                // C writes into the union of a non-map node; only maps carry commands
                Some(_) => {
                    let (mut p, mut l) = (String::new(), String::new());
                    if !parse_command_section(lx, &mut p, &mut l) {
                        return false;
                    }
                }
                None => {
                    log.push("ERROR: map command section with no associated map".into());
                    return false;
                }
            }
            continue;
        }
        if tok == "}" {
            return true;
        }
        if mr.nodes.len() == MAX_MAP_ROTATION_MAPS {
            log.push(format!("ERROR: maximum number of maps in one rotation ({MAX_MAP_ROTATION_MAPS}) reached"));
            return false;
        }
        match parse_node(lx, &tok, false, log) {
            Some(n) => mr.nodes.push(n),
            None => return false,
        }
    }
}

impl MapRotations {
    /// `G_ParseMapRotationFile` (+ the validation pass). On error the
    /// rotations parsed so far are kept, as in the C.
    pub fn parse(&mut self, text: &str, map_exists: &dyn Fn(&str) -> bool) -> bool {
        if text.is_empty() || text.len() >= 20000 - 1 {
            self.log.push(format!("ERROR: map rotation file is {}", if text.is_empty() { "empty" } else { "too long" }));
            return false;
        }
        let mut lx = Lexer { s: text.as_bytes(), p: 0 };
        let mut name: Option<String> = None;
        loop {
            let tok = lx.parse();
            if tok.is_empty() {
                break;
            }
            if tok == "{" {
                let Some(n) = name.take() else {
                    self.log.push("ERROR: unnamed map rotation".into());
                    return false;
                };
                if self.rotations.iter().any(|r| r.name == n) {
                    self.log.push(format!("ERROR: a map rotation is already named {n}"));
                    return false;
                }
                if self.rotations.len() == MAX_MAP_ROTATIONS {
                    self.log.push(format!("ERROR: maximum number of map rotations ({MAX_MAP_ROTATIONS}) reached"));
                    return false;
                }
                let mut mr = Rotation { name: n.clone(), nodes: Vec::new() };
                if !parse_rotation(&mut lx, &mut mr, &mut self.log) {
                    self.log.push(format!("ERROR: failed to parse map rotation {n}"));
                    return false;
                }
                self.rotations.push(mr);
                continue;
            }
            if name.is_none() {
                name = Some(tok);
            } else {
                self.log.push("ERROR: map rotation already named".into());
                return false;
            }
        }
        for i in 0..self.rotations.len() {
            let mut map_count = 0;
            for node in &self.rotations[i].nodes {
                let mut n = node;
                match n {
                    Node::Map { name, .. } => {
                        map_count += 1;
                        if !map_exists(name) {
                            self.log.push(format!("ERROR: rotation map \"{name}\" doesn't exist"));
                            return false;
                        }
                        continue;
                    }
                    Node::Return | Node::Label(_) => continue,
                    _ => {}
                }
                while let Node::Condition { target, .. } = n {
                    n = target;
                }
                if let Node::Goto(l) | Node::Resume(l) = n {
                    if !self.label_exists(i, l) && !self.rotations.iter().any(|r| &r.name == l) {
                        self.log.push(format!("ERROR: goto destination named \"{l}\" doesn't exist"));
                        return false;
                    }
                }
            }
            if map_count == 0 {
                self.log.push(format!("ERROR: rotation \"{}\" needs at least one map entry", self.rotations[i].name));
                return false;
            }
        }
        self.nodes = vec![0; self.rotations.len()];
        true
    }

    /// `G_LabelExists`.
    fn label_exists(&self, rotation: usize, name: &str) -> bool {
        self.rotations[rotation].nodes.iter().any(|n| match n {
            Node::Label(l) => l.eq_ignore_ascii_case(name),
            Node::Map { name: m, .. } => m.eq_ignore_ascii_case(name),
            _ => false,
        })
    }

    fn node_after(&self, current: usize, rotation: usize) -> usize {
        (current + 1) % self.rotations[rotation].nodes.len()
    }

    fn set_current_node(&mut self, node: usize, rotation: usize) {
        if self.nodes.len() < self.rotations.len() {
            self.nodes.resize(self.rotations.len(), 0);
        }
        self.nodes[rotation] = node;
    }

    /// `G_MapRotationActive`.
    pub fn active(&self) -> bool {
        self.current != NOT_ROTATING
    }

    /// `G_StopMapRotation`.
    pub fn stop(&mut self) {
        self.current = NOT_ROTATING;
    }

    /// `G_ClearRotationStack`.
    pub fn clear_stack(&mut self) {
        self.stack.clear();
    }

    /// `G_EvaluateMapCondition` (a chain of conditions ANDs).
    fn evaluate(lhs: Var, op: Op, num: i32, win: Team, target: &Node, ctx: &mut Ctx) -> (bool, Node) {
        let result = match lhs {
            Var::Random => (ctx.rand)() / (crate::kernel::rng::RAND_MAX / 2 + 1) != 0,
            Var::NumClients => match op {
                Op::Lt => ctx.num_connected < num,
                Op::Gt => ctx.num_connected > num,
                Op::Eq => ctx.num_connected == num,
            },
            Var::LastWin => ctx.last_win == Some(win),
        };
        if let Node::Condition { lhs, op, num_clients, last_win, target: t2 } = target {
            let (r2, n) = Self::evaluate(*lhs, *op, *num_clients, *last_win, t2, ctx);
            return (result && r2, n);
        }
        (result, target.clone())
    }

    /// `G_GotoLabel`.
    fn goto_label(&mut self, rotation: usize, mut node_index: usize, name: &str, reset_index: bool, depth: i32, ctx: &mut Ctx) -> bool {
        // search the rotation names...
        if self.start(name, true, true, reset_index, depth, ctx) {
            return true;
        }
        // ...then labels in the rotation
        let n = self.rotations[rotation].nodes.len();
        for i in 0..n {
            if let Node::Label(l) = &self.rotations[rotation].nodes[i] {
                if l.eq_ignore_ascii_case(name) {
                    let after = self.node_after(i, rotation);
                    self.set_current_node(after, rotation);
                    self.advance_depth(depth, ctx);
                    return true;
                }
            }
        }
        // finally a map by name
        for _ in 0..n {
            node_index = self.node_after(node_index, rotation);
            if let Node::Map { name: m, .. } = &self.rotations[rotation].nodes[node_index] {
                if m.eq_ignore_ascii_case(name) {
                    self.set_current_node(node_index, rotation);
                    self.advance_depth(depth, ctx);
                    return true;
                }
            }
        }
        false
    }

    /// `G_StepMapRotation`: true = keep stepping.
    fn step(&mut self, rotation: usize, node_index: usize, depth: i32, ctx: &mut Ctx) -> bool {
        let depth = depth + 1;
        let mut node = self.rotations[rotation].nodes[node_index].clone();
        let rname = self.rotations[rotation].name.clone();
        if depth > 32 && !matches!(node, Node::Map { .. }) {
            if depth > 64 {
                self.log.push(format!("ERROR: infinite loop protection stopped at map rotation {rname}"));
                return false;
            }
            self.log.push(format!("WARNING: possible infinite loop in map rotation {rname}"));
            return true;
        }
        loop {
            match node.clone() {
                Node::Condition { lhs, op, num_clients, last_win, target } => {
                    let (ok, target) = Self::evaluate(lhs, op, num_clients, last_win, &target, ctx);
                    if ok {
                        node = target;
                        continue;
                    }
                }
                Node::Return => {
                    let ret = if self.stack.is_empty() { -1 } else { self.stack.remove(0) };
                    if ret >= 0 {
                        let after = self.node_after(node_index, rotation);
                        self.set_current_node(after, rotation);
                        let name = self.rotations.get(ret as usize).map(|r| r.name.clone()).unwrap_or_default();
                        if self.start(&name, true, false, false, depth, ctx) {
                            return false;
                        }
                    }
                }
                Node::Map { name, post_command, layouts } => {
                    if (ctx.map_exists)(&name) {
                        let after = self.node_after(node_index, rotation);
                        self.set_current_node(after, rotation);
                        // G_IssueMapChange
                        self.pending = Some(MapChange { map: name, post_command, layouts });
                        return false;
                    }
                    self.log.push(format!("WARNING: skipped missing map {name} in rotation {rname}"));
                }
                Node::Label(_) => {}
                Node::Goto(l) | Node::Resume(l) => {
                    let reset = matches!(node, Node::Goto(_));
                    let after = self.node_after(node_index, rotation);
                    self.set_current_node(after, rotation);
                    if self.goto_label(rotation, node_index, &l, reset, depth, ctx) {
                        return false;
                    }
                    self.log.push(format!("WARNING: label, map, or rotation {l} not found in {rname}"));
                }
            }
            return true;
        }
    }

    fn advance_depth(&mut self, mut depth: i32, ctx: &mut Ctx) {
        let rotation = self.current;
        if rotation < 0 || rotation as usize >= self.rotations.len() {
            return;
        }
        let r = rotation as usize;
        let mut idx = self.nodes.get(r).copied().unwrap_or(0);
        if idx >= self.rotations[r].nodes.len() {
            self.log.push(format!("WARNING: index incorrect for map rotation {}, trying 0", self.rotations[r].name));
            idx = 0;
        }
        while self.step(r, idx, depth, ctx) {
            idx = self.node_after(idx, r);
            depth += 1;
        }
    }

    /// `G_AdvanceMapRotation`: the map change it issues, if any.
    pub fn advance(&mut self, ctx: &mut Ctx) -> Option<MapChange> {
        self.pending = None;
        self.advance_depth(0, ctx);
        self.pending.take()
    }

    /// `G_StartMapRotation`.
    pub fn start(&mut self, name: &str, advance: bool, put_on_stack: bool, reset_index: bool, depth: i32, ctx: &mut Ctx) -> bool {
        let Some(i) = self.rotations.iter().position(|r| r.name.eq_ignore_ascii_case(name)) else { return false };
        if put_on_stack && self.current >= 0 {
            self.stack.insert(0, self.current);
        }
        self.current = i as i32;
        if advance {
            if reset_index {
                self.set_current_node(0, i);
            }
            self.advance_depth(depth, ctx);
        }
        true
    }

    /// `G_StartMapRotation` without advancing (g_initialMapRotation / the
    /// `maprotation` server command, which also clears the stack first).
    pub fn begin(&mut self, name: &str) -> bool {
        let mut never = || 0;
        let mut ctx = Ctx { num_connected: 0, last_win: None, map_exists: &|_| true, rand: &mut never };
        self.start(name, false, true, false, 0, &mut ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_run(mr: &mut MapRotations, n: i32, win: Option<Team>) -> Option<String> {
        let mut r = 0;
        let mut rnd = || {
            r += 1;
            r
        };
        let mut ctx = Ctx { num_connected: n, last_win: win, map_exists: &|m| m != "missing", rand: &mut rnd };
        mr.advance(&mut ctx).map(|c| c.map)
    }

    #[test]
    fn stock_rotation_cycles() {
        let mut mr = MapRotations::default();
        assert!(mr.parse("rotation1\n{\n  karith\n  arachnid2\n  nexus6\n}\n", &|_| true));
        assert!(mr.begin("rotation1"));
        let seq: Vec<String> = (0..4).filter_map(|_| ctx_run(&mut mr, 1, None)).collect();
        assert_eq!(seq, ["karith", "arachnid2", "nexus6", "karith"]);
    }

    #[test]
    fn conditions_labels_and_nested_rotations() {
        let text = r#"
// comments are skipped
main
{
  atcs
  {
    g_suddenDeathTime 30
    layouts default
  }
  if numClients > 4 big
  if lastWin aliens #alienwin
  missing
  niveus
  goto main
  big
  #alienwin
  tremor
  return
}
big
{
  transit
  return
}
"#;
        let mut mr = MapRotations::default();
        assert!(mr.parse(text, &|_| true), "{:?}", mr.log);
        mr.begin("main");
        assert_eq!(ctx_run(&mut mr, 1, None).as_deref(), Some("atcs"));
        // numClients > 4: nested rotation "big", then return to main
        assert_eq!(ctx_run(&mut mr, 8, None).as_deref(), Some("transit"));
        assert_eq!(mr.current, 1);
        assert_eq!(ctx_run(&mut mr, 1, None).as_deref(), Some("niveus"));
        assert_eq!(mr.current, 0);
        assert!(mr.log.iter().any(|l| l.contains("skipped missing map missing")));
    }

    #[test]
    fn post_command_section() {
        let mut mr = MapRotations::default();
        // the command section reads whole lines, so its closing brace is on its own line
        assert!(mr.parse("r { atcs {\n g_suddenDeathTime 30\n timelimit 40\n }\n }", &|_| true));
        match &mr.rotations[0].nodes[0] {
            Node::Map { post_command, .. } => assert_eq!(post_command, "g_suddenDeathTime 30;timelimit 40\n"),
            n => panic!("{n:?}"),
        }
    }
}
