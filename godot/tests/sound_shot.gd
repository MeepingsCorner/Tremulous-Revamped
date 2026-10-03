extends Node
## The FMOD + Steam Audio sound path end to end: fires the rifle on atcs and
## prints s_fmodinfo. With TREM_FMOD_WAV=<file> FMOD writes its mix there.
## godot --path godot res://tests/sound_shot.tscn
var main: Node
var game

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	print("sound: load -> '", game.load_bsp(game.pk3_paths(), "atcs"), "'")
	main.get_node("HUD").close_menu()
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--occ="):
			game.exec("s_occlusion " + a.substr(6))
			print("sound: s_occlusion ", a.substr(6))
		if a.begins_with("--rev="):
			game.exec("s_reverb " + a.substr(6))
			print("sound: s_reverb ", a.substr(6))
	_run()

func _wait(s: float) -> void:
	await get_tree().create_timer(s).timeout

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
	print("sound: alive=", not game.get_hud().is_empty() and game.get_hud().alive, " after ", t, " s")
	await _wait(1.0)
	game.exec("s_fmodinfo")
	game.exec("+attack")
	await _wait(1.0)
	print("sound: while firing")
	game.exec("s_fmodinfo")
	await _wait(1.0)
	game.exec("-attack")
	game.exec("+button3")
	await _wait(0.3)
	game.exec("-button3")
	print("sound: after taunt")
	game.exec("s_fmodinfo")
	await _wait(1.5)
	game.exec("s_fmodinfo")
	get_tree().quit()
