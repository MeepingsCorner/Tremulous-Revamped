extends Node
## Ragdoll corpses (ragdoll.rs, enhanced mode): one class dies with a push;
## its ragdoll must exist, simulate, hide the original body, settle on the
## floor; F10 to vanilla must give back the original corpse where it was.
## godot --path godot res://tests/ragdoll_shot.tscn -- --team=humans --class=rifle
## -> "rag: PASS|FAIL ..." lines, user://rag_<class>_<n>.png, a SUMMARY line
var main: Node
var game
var team := "humans"
var klass := "rifle"
var fails := []
## the test's own camera on the body (the game's goes to the team menu / spectating after a death)
var cam: Camera3D
var cam_on := false

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--team="):
			team = a.substr(7)
		if a.begins_with("--class="):
			klass = a.substr(8)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.exec("r_enhanced 1")
	game.exec("r_ragdolls 1")
	print("rag: load -> '", game.load_bsp(game.pk3_paths(), "atcs"), "'")
	main.get_node("HUD").close_menu()
	_run()

func _wait(s: float) -> void:
	await get_tree().create_timer(s).timeout

func _check(name: String, ok: bool, detail: String) -> void:
	print("rag: ", "PASS " if ok else "FAIL ", name, ": ", detail)
	if not ok:
		fails.append(name)

func _process(_d: float) -> void:
	if cam_on and cam and not cam.current:
		cam.make_current()

## look at `at` from the side, `dist` away and a bit above
func _aim(at: Vector3, dist: float) -> void:
	if not cam:
		cam = Camera3D.new()
		add_child(cam)
	cam.global_position = at + Vector3(dist * 0.8, dist * 0.55, dist * 0.6)
	cam.look_at(at, Vector3.UP)
	cam_on = true
	cam.make_current()

func _shot(n: int) -> void:
	main.get_node("HUD").close_menu()
	await get_tree().process_frame
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png("user://rag_%s_%d.png" % [klass, n])

## the ragdolls as [ent, corpse, started, bones, Vector3]
func _info() -> Array:
	var out := []
	for l in game.ragdoll_info():
		var p: PackedStringArray = l.split(" ")
		out.append([int(p[0]), int(p[1]), int(p[2]), int(p[3]), Vector3(float(p[4]), float(p[5]), float(p[6]))])
	return out

func _run() -> void:
	await _wait(3.0)
	# cheats: an immediate "kill" (else 20 s), funds and stage for evolving
	game.set_cheats(true)
	var evolve := team == "aliens" and not klass in ["builder", "level0"]
	game.exec("team " + team)
	game.exec("class " + ("level0" if evolve else klass))
	var t := 0.0
	while t < 20.0:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			break
		await _wait(0.5)
		t += 0.5
	if evolve:
		# the advanced classes come by evolving: funds, then the
		# class; a big one needs room: then near the alien structures, in
		# turn, until it fits; the tyrant needs stage 3 (zero thresholds:
		# the stages advance on the next frames)
		game.exec("set g_alienStage2Threshold 0")
		game.exec("set g_alienStage3Threshold 0")
		await _wait(1.0)
		game.exec("give funds")
		await _wait(1.0)
		var base_class = str(game.get_hud().get("class_id", ""))
		game.exec("class " + klass)
		await _wait(3.0)
		if str(game.get_hud().get("class_id", "")) == base_class:
			var spots: Array = []
			for cn in ["team_alien_overmind", "team_alien_spawn", "team_alien_booster", "team_alien_acid_tube"]:
				for p in game.find_entities(cn):
					spots.append(p)
			for p in spots:
				for off in [Vector3(3, 0, 0), Vector3(-3, 0, 0), Vector3(0, 0, 3), Vector3(0, 0, -3), Vector3(4, 0, 4), Vector3(-4, 0, -4)]:
					var q: Vector3 = p + off
					game.exec("setviewpos %d %d %d 0" % [int(q.x * 32), int(-q.z * 32), int(q.y * 32 + 48)])
					await _wait(0.4)
					game.exec("class " + klass)
					await _wait(1.2)
					if str(game.get_hud().get("class_id", "")) != base_class:
						break
				if str(game.get_hud().get("class_id", "")) != base_class:
					break
		print("rag: evolved to class ", game.get_hud().get("class", "?"), " (", game.get_hud().get("center_print", ""), ")")
		await _wait(2.0)
	var alive: bool = not game.get_hud().is_empty() and game.get_hud().alive
	_check("spawned", alive, "%s %s after %.1f s" % [team, klass, t])
	var me: int = game.my_entity()
	# third person all along: our own body is drawn (in first person it never is,
	# vanilla or enhanced, and after a death the game goes back to first person)
	game.third_person = true
	game.exec("cg_thirdPerson 1")
	game.exec("cg_thirdPersonAngle 60")
	game.exec("cg_thirdPersonRange 160")
	await _wait(1.0)
	# die, pushed sideways and up (as a hit from the side would)
	game.ragdoll_test_push(me, Vector3(3.0, 1.2, 0.0))
	var death_at: Vector3 = _actor_origin(me)
	game.exec("kill")
	t = 0.0
	var r := []
	while t < 3.0 and r.is_empty():
		await get_tree().process_frame
		t += get_process_delta_time()
		r = _info()
	_check("a ragdoll appears at the death", not r.is_empty(), "after %.2f s" % t)
	if r.is_empty():
		_finish()
		return
	await _wait(0.1)
	r = _info()
	var start: Vector3 = r[0][4]
	_aim(start, 3.5 if r[0][3] >= 12 else 2.4)
	_check("it simulates with every bone", r[0][2] == 1 and r[0][3] >= 8, "started %d, %d bones" % [r[0][2], r[0][3]])
	_check("the original body is hidden", not game.actor_visible(me), "")
	await _shot(1)
	await _wait(0.6)
	await _shot(2)
	await _wait(2.0)
	var p1: Vector3 = _info()[0][4] if not _info().is_empty() else start
	await _shot(3)
	await _wait(1.0)
	var p2: Vector3 = _info()[0][4] if not _info().is_empty() else start
	print("rag: root bone start ", start, " after 2.7 s ", p1, " after 3.7 s ", p2)
	_check("it was pushed", Vector2(p1.x - start.x, p1.z - start.z).length() > 0.1, "moved %.2f across" % Vector2(p1.x - start.x, p1.z - start.z).length())
	# (the root bone of a tall class starts well above the floor: measured
	# from where we stood, through the floor would be far below it)
	var stood: float = death_at.z / 32.0
	_check("it stays on the floor", p2.y > stood - 1.5, "fell %.2f (start y %.2f, now %.2f, stood at %.2f)" % [start.y - p2.y, start.y, p2.y, stood])
	_check("it comes to rest", p2.distance_to(p1) < 0.1, "moved %.3f in the last second" % p2.distance_to(p1))
	# F10 to vanilla: the original corpse, where the server has it
	game.exec("r_enhanced 0")
	await _wait(0.4)
	for l in game.actor_info():
		print("rag: vanilla actor ", l)
	var body: Vector3 = _body_near(me, death_at)
	_check("vanilla: no ragdoll, the original corpse back where it died", _info().is_empty() and body.x < 1e8, "ragdolls %d, death at %s, our body drawn %s, nearest corpse %s" % [_info().size(), death_at, game.actor_visible(me), body])
	await _shot(4)
	game.exec("r_enhanced 1")
	await _wait(0.5)
	# (bots' corpses may lie around too: at least ours)
	var ours := 0
	for x in _info():
		if x[0] == me or x[1] == 1:
			ours += 1
	_check("enhanced again: a ragdoll from the corpse", ours >= 1, "ragdolls %d" % _info().size())
	await _shot(5)
	_finish()

## "ent <i> origin=(x,y,z) <key> ..." (Quake units)
func _parse_origin(l: String) -> Vector3:
	var a := l.find("origin=(")
	if a < 0:
		return Vector3(1e9, 1e9, 1e9)
	var b := l.find(")", a)
	var v := l.substr(a + 8, b - a - 8).split(",")
	return Vector3(float(v[0]), float(v[1]), float(v[2]))

func _actor_origin(ent: int) -> Vector3:
	for l in game.actor_info():
		if l.begins_with("ent %d " % ent):
			return _parse_origin(l)
	return Vector3(1e9, 1e9, 1e9)

## our drawn body (the dead player, or after the respawn timer its corpse
## entity, key c<class>) within 48 units of where we died; else 1e9
func _body_near(me: int, at: Vector3) -> Vector3:
	var best := Vector3(1e9, 1e9, 1e9)
	for l in game.actor_info():
		var w: PackedStringArray = l.split(" ")
		if w.size() < 4 or not l.contains("vis=true"):
			continue
		if not (w[1] == str(me) or w[3].begins_with("c")):
			continue
		var o := _parse_origin(l)
		if Vector2(o.x - at.x, o.y - at.y).length() < 48.0 and o.distance_to(at) < best.distance_to(at):
			best = o
	return best

func _finish() -> void:
	print("SUMMARY ragdoll_shot class=%s status=%s fails=%d %s" % [klass, "ok" if fails.is_empty() else "fail", fails.size(), ", ".join(fails)])
	cam_on = false
	game.third_person = false
	game.exec("cg_thirdPerson 0")
	game.exec("cg_thirdPersonAngle 0")
	game.exec("cg_thirdPersonRange 75")
	game.exec("r_enhanced 1")
	get_tree().quit()
