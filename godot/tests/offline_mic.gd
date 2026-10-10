extends Node
## The microphone test outside an online game (the editor's own ATCS
## start: no network): the K key, the real microphone, then a tone; the
## voice HUD (user://offline_mic.png).
## godot --path godot res://tests/offline_mic.tscn
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

func _run() -> void:
	await _wait(4.0)
	print("off: ", game.exec("set net_enabled 0\nset name Checker\ndevmap atcs"))
	while not (game.has_world() and not game.is_loading()):
		await _wait(0.5)
	await _wait(2.0)
	hud.close_menu()
	game.exec("team humans")
	await _wait(0.5)
	game.exec("class rifle")
	await _wait(3.0)
	hud.close_menu()
	await _wait(0.5)
	print("off: net ", game.net_status())
	await _key_k()
	await _wait(2.5)
	print("off: real mic ", game.voip_hud_state().get_slice(" power", 0), " | ", game.voip_hud_state().get_slice(" hud=", 1))
	game.voip_test_tone(220.0, 3000.0)
	await _key_k()
	await _wait(0.3)
	await _key_k()
	await _wait(1.5)
	print("off: tone ", game.voip_hud_state().get_slice(" hud=", 1))
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png("user://offline_mic.png")
	await _key_k()
	await _wait(0.5)
	print("off: after K ", game.voip_hud_state().get_slice(" power", 0))
	get_tree().quit(0)
