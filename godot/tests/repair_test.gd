extends Node
## Construction kit repair (CheckCkitRepair): a damaged human structure in
## the crosshair gains 18 health a second (HBUILD_HEALRATE every
## HBUILD_REPEAT ms), not all at once.
## godot --path godot res://tests/repair_test.tscn
## -> "repair: PASS|FAIL ..." lines and a SUMMARY line
var main: Node
var game
var fails := []

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	print("repair: load -> '", game.load_bsp(game.pk3_paths(), "atcs"), "'")
	main.get_node("HUD").close_menu()
	_run()

func _wait(s: float) -> void:
	await get_tree().create_timer(s).timeout

func _check(name: String, ok: bool, detail: String) -> void:
	print("repair: ", "PASS " if ok else "FAIL ", name, ": ", detail)
	if not ok:
		fails.append(name)

func _health(cn: String, ent: int) -> Array:
	for l in game.structure_health(cn):
		var w: PackedStringArray = l.split(" ")
		if int(w[0]) == ent:
			return [int(w[1]), int(w[2])]
	return [-1, -1]

func _run() -> void:
	await _wait(3.0)
	game.set_cheats(true)
	game.exec("team humans")
	game.exec("class ckit")
	var t := 0.0
	while t < 20.0:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			break
		await _wait(0.5)
		t += 0.5
	# the reactor: tall, near the human spawn
	var cn := "team_human_reactor"
	var list: PackedStringArray = game.structure_health(cn)
	if list.is_empty():
		cn = "reactor"
		list = game.structure_health(cn)
	_check("a human structure to repair", not list.is_empty(), "%s %s" % [cn, list])
	if list.is_empty():
		_finish()
		return
	var ent := int(list[0].split(" ")[0])
	var full := int(list[0].split(" ")[2])
	var at: Vector3 = game.find_entities(cn)[0]
	var q := Vector3(at.x * 32.0, -at.z * 32.0, at.y * 32.0)
	# stand 80 units away on each side in turn until the repair lands
	var gained := 0
	var start := 0
	for a in range(4):
		var yaw := a * 90.0
		var dir := Vector3(cos(deg_to_rad(yaw)), sin(deg_to_rad(yaw)), 0)
		var p := q - dir * 80.0
		game.exec("setviewpos %d %d %d %d" % [int(p.x), int(p.y), int(q.z), int(yaw)])
		await _wait(0.5)
		game.test_set_health(ent, full / 2)
		start = _health(cn, ent)[0]
		await _wait(3.0)
		gained = _health(cn, ent)[0] - start
		print("repair: side %d: %d -> %d" % [a, start, start + gained])
		if gained > 0:
			break
	_check("it repairs", gained > 0, "+%d in 3 s" % gained)
	_check("not all at once: about 18 a second", gained >= 18 and gained <= 72, "+%d in 3 s (full would be +%d)" % [gained, full - start])
	_finish()

func _finish() -> void:
	print("SUMMARY repair_test status=%s fails=%d %s" % ["ok" if fails.is_empty() else "fail", fails.size(), ", ".join(fails)])
	get_tree().quit()
