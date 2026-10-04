# Token method: Tremulous Godot + Rust port

**Status.** Fourth version, 4 October 2026. It specialises the *Methods: Token-Efficient Game Porting with AI Agents*, third edition (cited as §n), to this project, and it incorporates the owner's standing instructions and the evidence of this project's own sessions. It supersedes the earlier versions.

**Scope.** Agent sessions that read, change, build or verify this repository through the cloud workspace and the owner's Windows PC. Progress is not recorded here; it belongs in `sheets/`, the commit log and `reports/handoff/`.

**Conventions.** Rules carry identifiers (R1, R2, …) so that briefs and handoffs can cite a rule without restating it. **MUST** marks a rule whose breach invalidates the work. **SHOULD** marks the default, from which a departure requires a reason in `sheets/decisions.tsv`. Every claim of effect is graded:

| Grade | Standard |
|---|---|
| **[M]** measured | Observed in this project; the observation is named. |
| **[D]** documented | Stated by a tool's documentation or `--help`; its token effect has not been measured. |
| **[S]** strong | Follows from a well-understood mechanism, chiefly the price model of §1. |
| **[H]** hypothesis | Testable but untested; listed in §11. |

---

## 1. Cost model and its consequences (§8.1)

With base input price normalised to 1, a task of *T* turns costs approximately

> *C* ≈ Σₜ (0.1·*R*ₜ + *w*·*W*ₜ + *F*ₜ + 5·*O*ₜ)

where *R* is cached input read, *W* input written to cache (*w* = 1.25 for a five-minute cache, 2 for a one-hour cache), *F* uncached input and *O* output, including reasoning. The coefficients are vendor parameters and may change; the argument depends only on their order of magnitude. [S] Three consequences justify every rule that follows:

1. **Turns multiply context.** A context of *K* tokens costs at least 0.1·*K* per turn. Reducing the number of turns usually outweighs reducing the size of any single output.
2. **Cold writes dominate when they occur.** Rewriting a context costs 12.5–20 times a warm turn of the same size.
3. **Output is the dearest token.** Prose, regenerated files and deliberation on mechanical steps cost about five times their length as input.

The project's operating parameter is a one-hour cache. Waits shorter than an hour are therefore cheap; restarts are not. [S]

---

## 2. Standing instructions

| Id | Rule | Basis |
|---|---|---|
| R1 | Code is located and read through gamedb, via the guarded wrapper `tools/g` (§4). A plain-text search is permitted only for code the index cannot represent, and the reason is stated each time. | Owner |
| R2 | Each completed piece of work is committed; nothing is pushed. Commit messages end with the session's attribution lines. | Owner |
| R3 | An enhancement is an option whose default reproduces the original (`r_enhanced`, `r_rtMode`, `r_rtBackend`, `r_hdTextures`, `s_fmod`, `s_occlusion`) and has a `decisions.tsv` row. | Owner; §1.1 |
| R4 | A client-side addition MUST NOT affect joining vanilla 1.1.0 (protocol 69) or 1.3 (protocol 71) servers. Optional packs reside outside the pk3 search path. | Owner |
| R5 | The agent never closes the owner's Godot. A job that replaces a DLL in `godot/bin` waits for the editor alone (title ending " - Godot Engine", command line not naming `hdproj`), and the owner is asked to close it. The owner is told by exact window title which windows to close and which to keep. | Owner; [M] A broader title match mistook the console of the agent's own 4K build ("Godot Engine (Console)") for the editor and suspended the queue for 30 minutes; ambiguous instructions led the owner to close the runner instead. |
| R6 | Derived artefacts (the index, the `.hdtex` pack, generated code) are never canonical. On disagreement the artefact is rebuilt, not reasoned about. | §1.4 |

---

## 3. Execution topology

| Id | Rule | Evidence |
|---|---|---|
| R7 | Work runs where its tools are. The cloud workspace performs compile checks, unit tests, indexing and public-source research. The PC performs release builds, GPU work and Godot runs, through `tools/agent_runner.ps1` (`.agent/queue/<id>.ps1` → `.agent/out/<id>.log`, `<id>.done`). | [M] Cloud checks caught every compile error before a 2–4 minute PC build. |
| R7a | `cfg(windows)` Rust is checked in the cloud with `cargo check --target x86_64-pc-windows-gnu`, and the C shim with `ccheck`, before any PC build. | [M] Every Windows-only binding error of t027 was caught in the cloud; no PC build failed. |
| R8 | Any PC process that outlives its job is started with `Invoke-CimMethod Win32_Process Create` (job j075 is the template). `Start-Process` and `&` are not used for this. | [M] Three times, a child process inherited the runner's output pipe and suspended the queue; the third (the AudioSR build, j073) broke this rule from habit and held the queue for hours. |
| R9 | A copied Godot project contains `.godot/extension_list.cfg` naming `res://trem.gdextension`, and each test scene first asserts that a Rust class exists. | [M] Without the file the extension did not load, and every Rust class was reported "not declared". |
| R10 | Job scripts share one fixed form: wait for the editor → build → copy → test → filter → `SUMMARY`. A new job is derived by editing the previous one. | [S] A fixed form keeps its text in the cached prefix (§8.4). |
| R11 | Each long-running producer ends with one line, `SUMMARY <tool> status=<ok\|fail> errors=<n> warnings=<n> first=<pointer\|-> detail=<path>`. The agent reads that line and opens `detail` only on failure. | [S] §9.6 |

---

## 4. The index

`tools/g <command> ARGS` performs, in one call:

1. an incremental `gamedb index`;
2. a `stats` guard that exits with status 3 when the index is near-empty;
3. the query;
4. pointers stamped `path:start-end@gen`;
5. a footer when a result reaches its cap;
6. an explicit `no match` line in place of a silent empty result;
7. for an unindexed `read`, a pointer to the plain-text fallback.

| Id | Rule | Evidence |
|---|---|---|
| R12 | Index before every query sequence. The wrapper does this. | [M] After edits, searches returned nothing for functions that existed. |
| R13 | Locate with `search`, then `read` one body. A whole file is never opened to find a function (`menus.rs` is 380 KB). | [D] |
| R14 | A sequence of lookups is replaced by one `sql --json` statement. The `COUNT(*)` is taken first when the size of the result is unknown. | [D]; [S] §8.6.2 |
| R15 | Call graphs are candidate sets, not proof. The anchor is pinned with `--path`. | [D] Edges are resolved by name only. |
| R16 | After any parser change, coverage is measured: `fn` lines in each file against indexed rows, and the old index diffed against the new by `path::name`. A lost name is either a regression or a phantom entry, and is classified before acceptance. | [M] See below. |

**Parser coverage.** gamedb targets C, C++, C# and Java; this repository is predominantly Rust. Commit c3da5da of gamedb fixed three defects; the patch is kept in `tools/gamedb-patches/`, and the self-test passes 93 of 93.

- `&'static str` was lexed as a character literal, so the enclosing function absorbed the functions that followed it.
- `pub(crate) fn f(` was indexed as a function named `pub`.
- `__declspec(dllexport)` C exports were omitted.

[M] Before the fix, 70 of the 159 functions in `game.rs` were absent and 88 phantom `pub` entries existed. After it, the index gained 643 real functions across the tree, removed 90 phantom entries, and lost no real function.

There are 66 open exceptions out of 3,225 functions in `crates/`: functions named `new` (excluded by design as a C++ and Java keyword), `const fn`, return types containing `=`, and signatures split over several lines. Shader sources inside raw-string constants are not functions; they are read by line range.

---

## 5. Input economy (§8.2, §8.6)

| Id | Rule | Evidence |
|---|---|---|
| R17 | Command output is filtered at its source: build lines (`^error\|Finished\|error\[`), test lines (`test result\|FAILED`), and Godot output with known noise removed. Rust uses `--message-format=short`. | [M] An unfiltered Godot run produced about 100 lines and concealed the result. |
| R18 | A failure's `file:line` is resolved to its enclosing function, and only that range is read. | [S] §8.6.6 |
| R19 | Data questions are answered from `sheets/` (`re/symbols.tsv` maps original C functions to Rust modules and their status) before code is read. | [S] §1.3 |
| R20 | No directory is listed recursively without a scope. | [M] One listing returned 97,000 characters. |
| R21 | Reference material is fetched narrowly: one file through a sparse clone, or one documentation page reduced to the relevant terms. Public sources are cloned in the cloud, not staged from the PC. | [M] The Steam Audio parameter semantics came from one source file, after its documentation proved silent on them. |
| R22 | Only changed files cross between machines, in one call each way. Many small files are archived before staging. | [M] A 50-file stage echoed about 5–15k tokens of paths. |
| R22a | A file sent to the PC is staged under a name unique to that version (`<name>_v<job>.<ext>`), and its hash is checked by staging it back before any job depends on it. | [M] Repeated sends of one staging path delivered the previous version; three audio fixes were tested one version late before a missing diagnostic field exposed it. |
| R23 | Images are cropped, downscaled and combined into one contact sheet. An image is sent only when its pixel difference exceeds a threshold. | [S] An image costs about width × height / 750 tokens. |

---

## 6. Turn economy (§8.1, §8.6)

| Id | Rule | Evidence |
|---|---|---|
| R24 | Independent calls are issued in parallel within one turn. | [S] This saves (*n* − 1)·0.1·*K* in context re-reads. |
| R25 | A wait is one sleep of the expected duration (a release build is about 2.5 min; a build plus sound test about 5 min), followed by one check of `.done`. A wait never exceeds the cache lifetime, and status is never polled in a loop. | [S]; [M] Durations as stated. |
| R26 | A check, a fix and a re-check are folded into one job when the fix is mechanical. | [S] |
| R27 | Affected tests run during iteration (`cargo test -p <crate> <filter>`); the full suite runs before commit. | [S] §8.6.7 |
| R28 | A file just edited is not re-read, and unordered or timestamped output is never placed in reused text. | [S] |

---

## 7. Verification (§3, §4)

| Id | Rule | Evidence |
|---|---|---|
| R29 | A test proves that its stimulus occurred before its result is trusted (for example, the player spawned and the weapon fired). | [M] The sound test sent input through `game.console()`, which reaches the server console. Nothing fired, and the silence was misread as an audio fault. Client input goes through `game.exec()`. |
| R30 | Each subsystem exposes a numeric self-report that a test prints: `s_fmodinfo` (plays, failures, direct-simulation runs, mean occlusion, master peak); `TREM_FMOD_WAV` for the mixed output's peak and RMS; `r_rtDebug 1\|2\|3` to isolate one ray-traced term. | [M] One debug image located the flat-ambient-occlusion defect. |
| R30a | A fault inside a third-party native library is reproduced first in a cloud harness that links that library's Linux build with its symbols (`/tmp/bake`), under gdb. PC iterations start only after the harness passes. | [M] Steam Audio 4.8.1: three bake faults (a NULL progress callback called unchecked, a probe volume centred on the origin, `numVisSamples` 0) and one run-time fault (a probe batch loaded from disk and not committed) were each found in one harness run, after PC-only attempts had failed. |
| R30b | A native crash on the PC is located before it is guessed at: `TREM_CRASH_LOG` makes the shim log the faulting module + offset and the stack's return addresses, and the shim ships a PDB, which `llvm-symbolizer` reads in the cloud. | [M] The pathing crash resolved to `iplSimulatorRunPathing` in one run. |
| R30c | An A/B measurement uses a steady synthetic stimulus (white noise, `test:noise`), alternates the two states at least twice, and records the component's own state alongside the level. "No change" across several unrelated toggles is a measurement or stuck-state defect, never a weak effect. | [M] Gusty wind masked every audio effect. Steady noise and a spatializer readback then exposed two code defects: transmission type tied to the thickness switch, and frozen pathing outputs still applied after a source left the simulator. |
| R30d | Every GDScript test is parse-checked in the cloud (`Godot_v4.7.2 --headless --check-only --script`) before it is sent, and every PC test run has a time limit (`WaitForExit(600000)`, then end it). | [M] A type-inference error left an empty grey window holding the queue for 29 minutes; the cloud check reproduces that error. |
| R31 | Every port addition has unit tests, even where no original exists to compare against (BVH, mip chains, wrap padding, blend parsing, GPU → ray-tracing mode). | [S] |
| R32 | A failure is investigated, never waived; it is a defect in the game or in the test. | §4 |

---

## 8. Rust within Godot (§10)

| Id | Rule | Evidence |
|---|---|---|
| R33 | Pinned versions (Godot 4.7.2; godot-rust 0.5.5 with `api-4-7` and `experimental-godot-api`) appear in the stable prefix of every glue brief. | [S] An API signature recalled from an older release costs a compile retry. |
| R34 | The lowest compile tier that answers the question is used. Tier 0: `cargo check -p trem_sim`. Tier 1: affected `trem_sim` tests. Tier 2: `cargo check -p trem_godot`. These three run in the cloud. Tier 3: the release build and a headless scene on the PC, once per change set. | [S] |
| R35 | Engine API questions are answered from an index of `godot --headless --dump-extension-api`, or from `sheets/godot_api.tsv` (calls that have already compiled). Generated bindings and engine sources are never read for this purpose. | [H] |
| R36 | `.godot/`, `*.import`, `*.uid`, `target/` and the cargo registry are never read. Large `.tscn` files are never rewritten by hand. | [S] |
| R37 | `bind()` and `bind_mut()` guards are dropped before any engine call that may emit a signal or call back; re-entry goes through `call_deferred`. Panics remain `unwind`. | [S] §10.2 |

---

## 9. Context lifecycle (§8.5, §9.7, §9.9)

**Sub-agents.** A self-contained, read-heavy job (a wide survey, log triage, a screenshot sweep) goes to a fresh sub-agent. It writes details to a file and returns `task_id  verdict  fn_keys  pointers  detail_path  turns  tokens`, followed by at most 300 words. A sub-agent is never used for what one `tools/g` query answers. Mechanical steps run at the lowest reasoning effort that succeeds. [S]

**Handoff.** Work crossing a session boundary is carried by `reports/handoff/<task_id>.md`. Cloud workspaces are reclaimed, so a temporary directory is not durable.

- **Contents.** It holds references only:
  - the goal, the work in flight, and the decisions with their `decisions.tsv` ids;
  - the next step;
  - the skills to load;
  - the `fn_key`s and pointers;
  - the sheet rows touched;
  - the last commit hash and the highest green compile tier;
  - the last `SUMMARY` lines;
  - the failing tests;
  - the runner jobs still queued.
- **Never copied into it:** specifications, sheets, code, diffs, logs or this method.
- **Secrets** are redacted.
- **Acceptance.** It is under 5% of the session it replaces, and a fresh agent can state the next step and the files it will touch without opening anything else. [D for the practice; S for the effect]

**When to hand off.** A fresh session repays itself after

> *n* ≈ (1.25·*K*ₙₑw + 5·*H*) / (0.1·(*K*ₒₗd − *K*ₙₑw))

turns. For *K*ₒₗd = 150k, *K*ₙₑw = 20k and *H* = 2k, *n* ≈ 3.

**R38.** A handoff is written and a fresh session started when the context exceeds about four times its stable prefix and more than a few turns remain. A handoff is also written at each phase boundary: a feature verified, a merge, or a long PC job left running. A handoff is preferred to compaction, because its contents are governed by this contract rather than by a summariser. [S]

---

## 10. Reporting (§10, §11)

Reports lead with the result, then the evidence (a pointer, a command and its `SUMMARY` line, a test name), then the grade, then what was not checked. Messages to the owner are one or two sentences, sent only when something changed or a decision is needed. Templates and preambles stay fixed within a round so that they remain in the cached prefix.

---

## 11. Measurement and open questions

Each task appends to `sheets/tokens.tsv`: `task_id, agent, model, effort, turns, input_fresh, cache_read, cache_write, output, rows_verified, prefix_hash, retries, tool_calls`. After each round, the largest cost term is addressed first. A rise in cost has five recognised causes (§8.7): a changed prefix, a widened view, data growth outside the views, retries, or more turns.

| Question | Measurement | Rule |
|---|---|---|
| Does `tools/g` reduce turns per lookup? | Turns per located function, before and after; silent empties intercepted. | R12–R14 |
| Does parser coverage hold? | `fn` lines against indexed rows per file, after each gamedb change. | R16 |
| Does the detached launch keep the queue moving? | Jobs suspended by a child process per week; the target is 0. | R8 |
| Is the cache-read share adequate? | Cached against fresh input per task; the target is at least 90%. | R25, R28 |
| Does the handoff break-even hold here? | Tokens per remaining turn before and after a handoff; handoff size as a share of the session it replaced. | R38 |
| Does an engine-API index replace documentation reads? | API-related compile retries per glue task. | R35 |

**Limitations.** The [M] grades come from a small number of sessions on one project. They establish that each effect occurred, not its typical magnitude. Price coefficients and cache lifetimes are vendor parameters, and every [S] claim that depends on them would change with them. The gamedb grades describe commit c3da5da; the binary's `--help` takes precedence over this document.
