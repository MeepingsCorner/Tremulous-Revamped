extends Node
## Checks the voice / overmind / chaingun fixes in a listen server on ATCS:
## the real microphone's capture level, the local player in the voice HUD
## (a tone as the microphone), the voice page's Refresh, the overmind
## pulse in vanilla vs enhanced mode, and the chaingun in the enhanced
## mode (user://fixes_*.png).
## godot --path godot res://tests/fixes_check.tscn
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
	var path := "user://fixes_%s.png" % kind
	img.save_png(path)
	print("fixes: %s -> %s" % [kind, ProjectSettings.globalize_path(path)])

func _click(vx: float, vy: float) -> void:
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
	print("fixes: click %s/%s at %s" % [menu, item, c])
	await _click(c.x, c.y)
	await _wait(0.4)

## The port's pulse spheres (spawn_fx: SphereMesh children of TremGame).
func _spheres() -> int:
	var n := 0
	for c in game.get_children():
		if c is MeshInstance3D and c.mesh is SphereMesh:
			n += 1
	return n

## Damage the overmind (its health drops: EV_OVERMIND_ATTACK) and count
## the pulse spheres over the next frames.
func _hit_overmind(mode: String) -> void:
	print("fixes: gfx ", game.gfx(mode).split("\n")[-1])
	await _wait(1.0)
	var om: PackedStringArray = game.structure_health("team_alien_overmind")
	if om.is_empty():
		for cn in ["team_human_reactor", "team_alien_spawn", "team_human_spawn"]:
			print("fixes: %s %s" % [cn, game.structure_health(cn)])
		print("fixes: overmind MISSING")
		return
	var parts := om[0].split(" ")
	var ent := int(parts[0])
	var before := _spheres()
	game.test_set_health(ent, int(parts[1]) - 50)
	var most := before
	for i in 30:
		await get_tree().process_frame
		most = max(most, _spheres())
	print("fixes: overmind %s: spheres before=%d max after=%d -> %s" % [mode, before, most, "PULSE" if most > before else "no pulse"])

func _run() -> void:
	await _wait(1.0)
	print("fixes: ", game.exec("set net_enabled 1\nset net_port 30796\nset sv_hostname FixesCheck\nset name Talker\ndevmap atcs"))
	var t := 0.0
	while not (game.has_world() and not game.is_loading()):
		await _wait(0.5)
		t += 0.5
		if t > 200:
			print("fixes: no map")
			get_tree().quit(1)
			return
	hud.close_menu()
	await _wait(3.0)
	game.set_cheats(true)
	print("fixes: ", game.exec("team humans"))
	print("fixes: ", game.exec("class rifle"))
	var t2 := 0.0
	while t2 < 20.0:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			break
		await _wait(0.5)
		t2 += 0.5
	await _wait(1.0)
	hud.close_menu()
	print("fixes: cvar at start: ", game.exec("s_voipInputDevice"))
	# 1. the real microphone: levels over 3 s (silence still has a noise floor)
	print("fixes: input devices ", AudioServer.get_input_device_list(), " current ", AudioServer.input_device, " enable_input ", ProjectSettings.get_setting("audio/driver/enable_input"))
	print("fixes: toggle ", game.exec("voip_toggle"))
	var peak := 0.0
	for i in 15:
		await _wait(0.2)
		var s: String = game.voip_hud_state()
		var p := float(s.get_slice("power=", 1).get_slice(" ", 0))
		peak = max(peak, p)
	print("fixes: real mic: ", game.voip_hud_state())
	print("fixes: real mic peak power %.5f (talking floor 0.01)" % peak)
	print("fixes: toggle ", game.exec("voip_toggle"))
	await _wait(0.5)
	# 2. the local player talking (a tone stands in for the voice)
	game.voip_test_tone(220.0, 3000.0)
	print("fixes: toggle ", game.exec("voip_toggle"))
	await _wait(1.5)
	print("fixes: tone: ", game.voip_hud_state())
	await _shot("talking")
	print("fixes: toggle ", game.exec("voip_toggle"))
	await _wait(0.5)
	# 2b. a quiet voice (power ~0.1: under cl_voipVADThreshold 0.25) with
	# voice activation on and Mic Volume Loud (3): activation opens
	game.exec("set cl_voipUseVAD 1
set cl_voipVADThreshold 0.25
set cl_voipCaptureMult 3")
	game.voip_test_tone(220.0, 1465.0)
	print("fixes: toggle ", game.exec("voip_toggle"))
	await _wait(1.5)
	print("fixes: quiet VAD: ", game.voip_hud_state().get_slice(" mic_fx", 0), " ", game.voip_hud_state().get_slice(" hud=", 1))
	print("fixes: toggle ", game.exec("voip_toggle"))
	await _wait(0.5)
	game.voip_test_tone(0.0, 0.0)
	# 3. the overmind pulse: none in vanilla, the pulse in enhanced
	await _hit_overmind("vanilla")
	await _wait(11.0) # G_OvermindThink's 10 s between attack events
	await _hit_overmind("enhanced")
	# 4. the chaingun, enhanced mode
	# bought at the armoury (give has no weapons in 1.1)
	var arm: PackedVector3Array = game.find_entities("team_human_armoury")
	if arm.is_empty():
		print("fixes: armoury MISSING")
	else:
		var a := arm[0]
		print("fixes: ", game.exec("setviewpos %d %d %d 180" % [a.x * 32 + 90, -a.z * 32, a.y * 32 + 10]))
		await _wait(1.0)
		print("fixes: ", game.exec("give funds"))
		print("fixes: ", game.exec("sell weapons"))
		await _wait(0.5)
		print("fixes: ", game.exec("buy chaingun"))
		await _wait(2.0)
		print("fixes: hud weapon ", game.get_hud().get("weapon", "?"))
		print("fixes: ", game.exec("setviewpos %d %d %d 0" % [a.x * 32 + 90, -a.z * 32, a.y * 32 + 10]))
		await _wait(1.0)
	await _shot("chaingun_enhanced")
	game.gfx("vanilla")
	await _wait(1.0)
	await _shot("chaingun_vanilla")
	print("fixes: cvar before menus: ", game.exec("s_voipInputDevice"))
	# 5. the voice page's Refresh
	hud.open_menu("ingame")
	await _wait(0.5)
	await _press("ingame", "options")
	await _press("ingame_options", "systemBtn")
	await _press("ingame_options", "Net & Sound")
	await _press("ingame_options", "Voice Chat...")
	print("fixes: menu ", menus.current_menu())
	print("fixes: cvar on the page: ", game.exec("s_voipInputDevice"), " menus copy [", menus.get_cvar("s_voipInputDevice"), "]")
	await _shot("voice_page")
	await _press("port_voice", "refreshmic")
	await _wait(0.5)
	print("fixes: after refresh ", game.voip_hud_state())
	print("fixes: cvar after refresh: ", game.exec("s_voipInputDevice"), " menus copy [", menus.get_cvar("s_voipInputDevice"), "]")
	await _wait(1.0)
	print("fixes: refresh command: ", game.exec("voip_refreshdevices"))
	await _wait(0.5)
	get_tree().quit(0)
