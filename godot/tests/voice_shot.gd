extends Node
## The voice chat HUD and settings page (dec053), saved as user://voice_*.png:
## a listen server on ATCS, a 440 Hz tone as the microphone, voip_toggle
## (the K key) on and off, then Options > Net & Sound > Voice Chat...
## godot --path godot res://tests/voice_shot.tscn
var main: Node
var game
var menus
var hud

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.bots_per_team = 0
	hud = main.get_node("HUD")
	menus = main.get_node_or_null("HUD/TremMenus")
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	_run()

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _shot(kind: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	var img := get_viewport().get_texture().get_image()
	var path := "user://voice_%s.png" % kind
	img.save_png(path)
	print("voice_shot: %s -> %s" % [kind, ProjectSettings.globalize_path(path)])

func _click(vx: float, vy: float) -> void:
	# 640x480 virtual units to the window
	var size := get_viewport().get_visible_rect().size
	var p := Vector2(vx * size.x / 640.0, vy * size.y / 480.0)
	for pressed in [true, false]:
		var ev := InputEventMouseButton.new()
		ev.button_index = MOUSE_BUTTON_LEFT
		ev.position = p
		ev.global_position = p
		ev.pressed = pressed
		Input.parse_input_event(ev)
		await get_tree().process_frame
	var mv := InputEventMouseMotion.new()
	mv.position = p
	mv.global_position = p
	Input.parse_input_event(mv)
	await get_tree().process_frame

func _press(menu: String, item: String) -> void:
	var c: Vector2 = menus.item_center(menu, item)
	print("voice_shot: click %s/%s at %s" % [menu, item, c])
	await _click(c.x, c.y)
	await _wait(0.4)

func _run() -> void:
	await _wait(1.0)
	print("voice_shot: ", game.exec("set net_enabled 1\nset net_port 30795\nset sv_hostname VoiceShot\nset name Talker\ndevmap atcs"))
	var t := 0.0
	while not (game.has_world() and not game.is_loading()):
		await _wait(0.5)
		t += 0.5
		if t > 200:
			print("voice_shot: no map")
			get_tree().quit(1)
			return
	hud.close_menu()
	await _wait(3.0)
	game.set_cheats(true)
	print("voice_shot: ", game.exec("team humans"))
	print("voice_shot: ", game.exec("class rifle"))
	var t2 := 0.0
	while t2 < 20.0:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			break
		await _wait(0.5)
		t2 += 0.5
	await _wait(1.0)
	hud.close_menu()
	game.voip_test_tone(440.0, 8000.0)
	print("voice_shot: toggle ", game.exec("voip_toggle"))
	await _wait(1.5)
	print("voice_shot: status ", game.voip_hud_state())
	await _shot("hud_on")
	print("voice_shot: toggle ", game.exec("voip_toggle"))
	await _wait(0.5)
	print("voice_shot: status ", game.voip_hud_state())
	await _shot("hud_off")
	# the settings page
	hud.open_menu("ingame")
	await _wait(0.5)
	await _press("ingame", "options")
	await _press("ingame_options", "systemBtn")
	await _press("ingame_options", "Net & Sound")
	await _shot("sound_panel")
	await _press("ingame_options", "Voice Chat...")
	print("voice_shot: menu ", menus.current_menu())
	await _shot("voice_page")
	get_tree().quit(0)
