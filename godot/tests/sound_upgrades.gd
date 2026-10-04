extends Node
## The enhanced sound upgrades, measured (fmodaudio.rs / trem_audio.c):
## pathing (scan + an audible A/B), wall thickness, per-source reflections,
## air absorption, doppler, headphones / speakers, the CPU budget, and the
## vanilla mode left as it was. Prints "upg: ..." lines and a SUMMARY.
## godot --path godot res://tests/sound_upgrades.tscn [-- --map=atcs]
var main: Node
var game
var fails := []
const LOOP := "test:noise"

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	var map := "atcs"
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--map="):
			map = a.substr(6)
	game.exec("r_enhanced 1")
	game.exec("s_pathing 1")
	game.exec("s_occlusion -1")
	print("upg: load -> '", game.load_bsp(game.pk3_paths(), map), "'")
	main.get_node("HUD").close_menu()
	_run()

func _wait(s: float) -> void:
	await get_tree().create_timer(s).timeout

func _check(name: String, ok: bool, detail: String) -> void:
	print("upg: ", "PASS " if ok else "FAIL ", name, ": ", detail)
	if not ok:
		fails.append(name)

## mean output RMS of voice `v` over `s` seconds
func _rms(v: int, s: float) -> float:
	game.fmod_meter(v)
	await _wait(s)
	return game.fmod_meter_read()

## the spatializer's own state for voice `v`
func _diag(label: String, v: int) -> void:
	var i: PackedFloat32Array = game.fmod_voice_info(v)
	print("upg: diag %s: occ %.2f (dsp %.2f) trans dsp %.3f/%.3f/%.3f type/air/path %d, thick %.2f, pathmix %.2f, xsrc %d, path eq %.3f sh0 %.4f, refl ir %d apply %d, dsp in %.4f out %.4f" % [label, i[1], i[12], i[13], i[14], i[15], int(i[16]), i[3], i[2], int(i[5]), i[20], i[21], int(i[22]), int(i[23]), i[17], i[18]])

func _cam() -> Transform3D:
	var c: Camera3D = game.view_camera()
	return c.global_transform if c else Transform3D()

func _run() -> void:
	await _wait(3.0)
	game.exec("team humans")
	game.exec("class rifle")
	var t := 0.0
	while t < 20.0:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			break
		await _wait(0.5)
		t += 0.5
	# the bake (or the cache)
	t = 0.0
	while t < 240.0 and not String(game.fmod_stats()).contains("pathing ready"):
		await _wait(1.0)
		t += 1.0
	var st: String = game.fmod_stats()
	_check("pathing ready", st.contains("pathing ready"), "after %.0f s" % t)
	await _wait(1.0)

	# the listener stays put from here on (the player may be moved or killed)
	game.fmod_lock_listener(true)
	var cam0 := _cam()  # where the listener is frozen
	await _wait(0.2)
	# --- pathing: the scan around the listener
	var scan: String = game.fmod_path_scan(25.0)
	print("upg: ", scan)
	var re := RegEx.create_from_string("visible (\\d+) \\(path (\\d+), mean angle ([\\d.]+) deg\\); blocked (\\d+) \\(path (\\d+), mean angle ([\\d.]+) deg")
	var m := re.search(scan)
	if m:
		var vis := int(m.get_string(1)); var vis_ang := float(m.get_string(3))
		var blk := int(m.get_string(4)); var blk_path := int(m.get_string(5)); var blk_ang := float(m.get_string(6))
		_check("visible sources: the path is the straight line", vis > 0 and vis_ang < 15.0, "%d visible, mean %.1f deg" % [vis, vis_ang])
		_check("blocked sources get a path around", blk > 0 and blk_path * 2 >= blk, "%d of %d blocked" % [blk_path, blk])
		_check("blocked paths come from another direction", blk_ang > 20.0, "mean %.1f deg off the straight line" % blk_ang)
	else:
		_check("pathing scan", false, scan)

	# --- pathing A/B: a sound behind a wall, pathing on / off
	var best: Vector3 = game.fmod_path_best()
	var v: int = game.fmod_test_play(LOOP, best, true, false)
	await _wait(1.0)
	var on1 := await _rms(v, 2.0)
	_diag("hidden, pathing on", v)
	var info: PackedFloat32Array = game.fmod_voice_info(v)
	game.exec("s_pathing 0")
	await _wait(0.5)
	var off := await _rms(v, 2.0)
	_diag("hidden, pathing off", v)
	game.exec("s_pathing 1")
	await _wait(0.5)
	var on2 := await _rms(v, 2.0)
	print("upg: hidden source at ", best, ": occlusion %.2f, pathing mix %.2f, thickness %.2f m, transmission %.3f" % [info[1], info[2], info[3], info[4]])
	_check("pathing makes a hidden sound audible", on1 > off * 2.0 and on2 > off * 2.0, "rms on %.5f / off %.5f / on %.5f (%.1f dB)" % [on1, off, on2, 20.0 * log((on1 + on2) * 0.5 / max(off, 1e-9)) / log(10.0)])

	# --- wall thickness on the same hidden source (pathing off: the wall alone)
	game.exec("s_pathing 0")
	var thin := 0.0
	var thick := 0.0
	for k in 2:
		game.exec("s_wallThickness 0")
		await _wait(0.4)
		thin += await _rms(v, 2.0)
		if k == 0: _diag("thickness off", v)
		game.exec("s_wallThickness 1")
		await _wait(0.4)
		thick += await _rms(v, 2.0)
		if k == 0: _diag("thickness on", v)
	info = game.fmod_voice_info(v)
	_check("wall thickness muffles through solid", info[3] > 0.0 and thick < thin * 0.85, "%.2f m of solid, rms %.5f without / %.5f with (%.1f dB)" % [info[3], thin / 2, thick / 2, 20.0 * log(thick / max(thin, 1e-9)) / log(10.0)])
	game.exec("s_pathing 1")
	game.fmod_test_stop(v)

	# --- a visible source: no pathing doubling
	var c := cam0
	var front := c.origin - c.basis.z * 3.0
	v = game.fmod_test_play(LOOP, front, true, false)
	await _wait(1.0)
	var vis_on := await _rms(v, 1.5)
	_diag("visible 3 m", v)
	info = game.fmod_voice_info(v)
	game.exec("s_pathing 0")
	await _wait(0.5)
	var vis_off := await _rms(v, 1.5)
	game.exec("s_pathing 1")
	_check("a visible sound is not doubled by pathing", info[2] < 0.2 and absf(vis_on - vis_off) < 0.25 * vis_off, "pathing mix %.2f, rms %.5f on / %.5f off" % [info[2], vis_on, vis_off])

	# --- air absorption (the farthest point in sight: no wall, no path)
	var far: Vector3 = game.fmod_path_far()
	print("upg: farthest visible point ", far, " at %.1f m" % far.distance_to(c.origin))
	game.fmod_test_move(v, far)
	var air0 := 0.0
	var air1 := 0.0
	for k in 2:
		game.exec("s_airAbsorption 0")
		await _wait(0.4)
		air0 += await _rms(v, 2.0)
		if k == 0: _diag("air off", v)
		game.exec("s_airAbsorption 1")
		await _wait(0.4)
		air1 += await _rms(v, 2.0)
		if k == 0: _diag("air on", v)
	_check("air absorption takes off highs at a distance", air1 < air0 * 0.97, "rms %.5f off / %.5f on (%.1f dB)" % [air0 / 2, air1 / 2, 20.0 * log(air1 / max(air0, 1e-9)) / log(10.0)])

	# --- headphones / speakers
	game.exec("s_hrtf 0")
	await _wait(0.3)
	var fs0 := String(game.fmod_stats())
	game.exec("s_hrtf 1")
	await _wait(0.3)
	var fs1 := String(game.fmod_stats())
	var fr := RegEx.create_from_string("features (\\d+)")
	var f0 := int(fr.search(fs0).get_string(1)); var f1 := int(fr.search(fs1).get_string(1))
	_check("speakers / headphones switch", (f0 & 1) == 0 and (f1 & 1) == 1, "features %d -> %d" % [f0, f1])

	# --- doppler: towards, away (20 units/s), still
	var d_to := await _doppler(v, c, 1.0)
	var d_away := await _doppler(v, c, -1.0)
	game.fmod_test_move(v, c.origin - c.basis.z * 6.0)
	await _wait(0.8)
	var d_still: float = game.fmod_voice_info(v)[0]
	_check("doppler: higher approaching, lower leaving", d_to > 1.03 and d_away < 0.97 and absf(d_still - 1.0) < 0.01, "towards %.3f, away %.3f, still %.3f (expected ~1.062 / 0.945 / 1)" % [d_to, d_away, d_still])
	game.fmod_test_stop(v)

	# --- per-source reflections (a loud sound, reflections on / off)
	v = game.fmod_test_play(LOOP, c.origin - c.basis.z * 3.0, true, true)
	await _wait(1.5)
	var r1 := await _rms(v, 1.5)
	_diag("loud, reflections on", v)
	var x_in: float = game.fmod_voice_info(v)[5]
	game.exec("s_reverb 0")
	await _wait(1.6)
	var r0 := await _rms(v, 1.5)
	_diag("loud, reflections off", v)
	game.exec("s_reverb 1")
	_check("per-source reflections add the room", x_in > 0.5 and r1 > r0 * 1.03, "rms %.5f with / %.5f without" % [r1, r0])
	game.fmod_test_stop(v)

	# --- the CPU budget under load (firing, plus 30 loops around)
	var many := []
	for i in 30:
		var a := TAU * i / 30.0
		many.append(game.fmod_test_play(LOOP, c.origin + Vector3(cos(a), 0.0, sin(a)) * (4.0 + i % 5 * 3.0), true, i % 3 == 0))
	game.exec("+attack")
	await _wait(10.0)  # long enough for the budget to settle
	game.exec("-attack")
	info = game.fmod_voice_info(many[0])
	print("upg: cpu under load: direct %.2f ms (max %.2f), reflections+pathing %.1f ms (max %.1f), quality %d (direct + 10 x reflections), rays %d" % [info[6], info[7], info[8], info[9], info[10], info[11]])
	# the worker rests 3x a pass: its share of two threads is pass / (4 * pass)
	var busy: float = 100.0 * info[8] / maxf(info[8] * 4.0, 90.0 + info[8])
	_check("CPU within budget", info[6] < 2.5 and busy <= 30.0 and info[8] < 60.0, "direct %.2f ms/frame, reflection pass %.1f ms (worker busy %.0f%%)" % [info[6], info[8], busy])
	for w in many:
		game.fmod_test_stop(w)

	# --- vanilla: none of it
	game.exec("r_enhanced 0")
	await _wait(0.5)
	v = game.fmod_test_play(LOOP, best, true, false)
	game.fmod_test_move(v, best)
	await _wait(1.0)
	info = game.fmod_voice_info(v)
	var fv := int(fr.search(String(game.fmod_stats())).get_string(1))
	_check("vanilla mode: binaural only, no pathing / doppler / thickness", fv == 1 and info[2] == 0.0 and info[5] == 0.0 and absf(info[0] - 1.0) < 0.001, "features %d, pathing mix %.2f, extra source %d, doppler %.3f" % [fv, info[2], int(info[5]), info[0]])
	game.fmod_test_stop(v)
	game.exec("r_enhanced 1")

	game.fmod_lock_listener(false)
	print("upg: ", game.fmod_stats())
	print("SUMMARY sound_upgrades status=%s fails=%d %s" % ["ok" if fails.is_empty() else "fail", fails.size(), ", ".join(fails)])
	game.exec("s_pathing 1")
	game.exec("s_reverb 1")
	game.exec("s_hrtf 1")
	game.exec("s_airAbsorption 1")
	game.exec("s_wallThickness 1")
	game.exec("s_occlusion -1")
	game.exec("r_enhanced 1")
	get_tree().quit()

## the doppler factor of voice `v` moving along the view axis at 20 units/s
## (dir 1 towards the listener, -1 away)
func _doppler(v: int, c: Transform3D, dir: float) -> float:
	var f := 0.0
	var n := 0
	var t := 0.0
	while t < 1.0:
		var d := 24.0 - 20.0 * t if dir > 0.0 else 4.0 + 20.0 * t
		game.fmod_test_move(v, c.origin - c.basis.z * d)
		await get_tree().process_frame
		var dt := get_process_delta_time()
		t += dt
		if t > 0.4:
			f += game.fmod_voice_info(v)[0]
			n += 1
	return f / max(n, 1)
