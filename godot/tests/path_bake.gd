extends Node
## Builds (or loads) one map's sound pathing data and checks it:
## godot --path godot res://tests/path_bake.tscn -- --map=atcs
## Prints "bake: map=... state=... probes=... seconds=... fault=..." and the
## pathing scan around the human spawn.
var main: Node
var game

func _ready() -> void:
	var map := "atcs"
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--map="):
			map = a.substr(6)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.exec("r_enhanced 1")
	game.exec("s_pathing 1")
	print("bake: load ", map, " -> '", game.load_bsp(game.pk3_paths(), map), "'")
	main.get_node("HUD").close_menu()
	_run(map)

func _wait(s: float) -> void:
	await get_tree().create_timer(s).timeout

func _run(map: String) -> void:
	await _wait(2.0)
	game.exec("team humans")
	game.exec("class rifle")
	var t := 0.0
	var st := ""
	var re := RegEx.create_from_string("pathing (\\w+) \\((\\d+) probes, bake (-?\\d+)/1000, stage (\\d+), fault (\\d+)")
	var m: RegExMatch = null
	while t < 900.0:
		st = game.fmod_stats()
		m = re.search(st)
		if m and m.get_string(1) in ["ready", "failed", "off"]:
			break
		await _wait(1.0)
		t += 1.0
	if m:
		print("bake: map=%s state=%s probes=%s seconds=%.0f stage=%s fault=%s" % [map, m.get_string(1), m.get_string(2), t, m.get_string(4), m.get_string(5)])
	else:
		print("bake: map=%s state=unknown stats=%s" % [map, st])
	await _wait(1.0)
	print("bake: ", game.fmod_path_scan(25.0))
	get_tree().quit()
