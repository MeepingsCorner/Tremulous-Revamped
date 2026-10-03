# Handoff t026: FMOD silence, 4K pack, gamedb coverage

**Goal.** In-game sound through FMOD + Steam Audio; the 4K texture pack built and toggled by F10; all of it verified and committed (R2).

**Load.** The method: `docs/TOKEN_METHOD.md`. Index through `tools/g` (R1, R12).

**Last commit.** 45ead86. Uncommitted:

- `crates/trem_godot/native/trem_audio.c`
- `crates/trem_godot/src/bspview.rs` (SKY_SHADER `face_hd`)
- `godot/tests/sound_shot.gd`
- `docs/TOKEN_METHOD.md`
- `tools/g`
- `tools/gamedb-patches/`

The owner's `godot/main.tscn` and `re/source` changes are excluded.

**Highest green tier.** 3 (j023: release build and sound test ran).

**In flight.**

- **j025** (queued; waits for the Godot editor to close). It rebuilds the shim:
  - user-defined occlusion and transmission, starting at 1.0 (`voice_direct`);
  - linear distance from 80 to 1330 units, as `snd_dma.c S_SpatializeOrigin`;
  - `s_fmodinfo` gains `direct runs`, `mean occlusion` and `master peak`.

  Accept if the `while firing` line shows master peak > 0.1 and the WAV RMS is far above 2.7.
- **HD build.** A detached Godot (pid from j024) is upscaling 939 images. Progress is in `.agent/out/hd.log`, output in `user://hd`.

**Last SUMMARY-equivalent (j023).** Sounds 13/0 loaded, plays 186/0, voices 12, WAV peak 87, RMS 2.66: silent. Cause: the spatializer read occlusion and transmission from simulation outputs that were 0 until simulated (Steam Audio `fmod/src/spatialize_effect.cpp` `getDirectParams`).

**Next steps.**

1. Read `j025.log`.
2. If the sound passes, run the full tests and `gamedb index`, then commit (R2).
3. When `hd.log` reports completion, run in game with F10 and check VRAM and `r_hdMaxSize`.
4. Check the turret-hit blend visually.

**Open, not requested.** Turret walk-through prediction (`apply_entity` owner); the LAN `net_enabled` default.

**gamedb.** Commit c3da5da in the cloud clone `/home/claude/gamedb`, not pushed; the patch is in `tools/gamedb-patches/`. The installed `/usr/local/bin/gamedb` is the fixed build.
