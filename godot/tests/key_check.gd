extends Node
## The K key (bind k voip_toggle) as a real key event in a listen server
## on ATCS: in game, then with the team menu open; the microphone's state.
## godot --path godot res://tests/key_check.tscn
var main: Node
var game
var hud

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.bots_per_team = 0
	hud = main.get_node("HUD")
	_run()

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _key_k() -> void:
	for pressed in [true, false]:
		var ev := InputEventKey.new()
		ev.keycode = KEY_K
		ev.physical_keycode = KEY_K
		ev.unicode = 107
		ev.pressed = pressed
		Input.parse_input_event(ev)
		await get_tree().process_frame
		await get_tree().process_frame

func _state(what: String) -> void:
	print("key: %s: %s" % [what, game.voip_hud_state().get_slice(" power", 0)])

func _run() -> void:
	await _wait(1.0)
	game.exec("set net_enabled 1\nset net_port 30802\nset name Checker\ndevmap atcs")
	while not (game.has_world() and not game.is_loading()):
		await _wait(0.5)
	await _wait(2.0)
	print("key: bound to k: ", game.keys_for("voip_toggle"))
	_state("team menu up")
	await _key_k()
	await _wait(0.5)
	_state("K with the team menu up")
	hud.close_menu()
	await _wait(0.5)
	game.exec("team humans")
	game.exec("class rifle")
	await _wait(3.0)
	hud.close_menu()
	await _wait(0.5)
	_state("in game")
	await _key_k()
	await _wait(2.5)
	_state("K in game")
	await _key_k()
	await _wait(0.5)
	_state("K again")
	get_tree().quit(0)
