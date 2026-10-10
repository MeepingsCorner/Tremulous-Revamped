extends Node
## Classic (vanilla) mode on ATCS: models without / with the port's sun
## (user://sun_<SUN_TAG>_*.png): the reactor, the armoury, a human spawn
## with a bot on it, and the first-person rifle; the sun's visibility.
## SUN_TAG=before godot --path godot res://tests/sun_check.tscn
var main: Node
var game
var hud
var tag := "run"

func _ready() -> void:
	if OS.get_environment("SUN_TAG") != "":
		tag = OS.get_environment("SUN_TAG")
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.bots_per_team = 0
	hud = main.get_node("HUD")
	_run()

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _shot(kind: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	var path := "user://sun_%s_%s.png" % [tag, kind]
	get_viewport().get_texture().get_image().save_png(path)
	print("sun: %s -> %s" % [kind, ProjectSettings.globalize_path(path)])
	print("sun: voip ", game.voip_hud_state().get_slice(" power", 0), " vad=", game.exec("cl_voipUseVAD").strip_edges())

func _suns() -> String:
	var n := 0
	var vis := 0
	for c in game.get_children():
		if c is DirectionalLight3D:
			n += 1
			if c.is_visible_in_tree():
				vis += 1
	return "%d of %d" % [vis, n]

func _look_at(classname: String, dist: float, up: float, name: String) -> void:
	var e: PackedVector3Array = game.find_entities(classname)
	if e.is_empty():
		print("sun: no ", classname)
		return
	var a := e[0]
	game.exec("setviewpos %d %d %d 180" % [a.x * 32 + dist, -a.z * 32, a.y * 32 + up])
	await _wait(1.2)
	await _shot(name)

func _run() -> void:
	await _wait(1.0)
	game.exec("set net_enabled 1\nset net_port 30799\nset name Checker\ndevmap atcs")
	while not (game.has_world() and not game.is_loading()):
		await _wait(0.5)
	hud.close_menu()
	await _wait(2.0)
	game.set_cheats(true)
	game.exec("team humans")
	game.exec("class rifle")
	await _wait(3.0)
	hud.close_menu()
	game.add_bots("humans", 1)
	await _wait(1.0)
	game.gfx("vanilla")
	await _wait(1.0)
	print("sun: vanilla sun visible ", _suns())
	game.exec("cg_drawGun 1\nnotarget")
	await _look_at("team_human_reactor", 220, 0, "reactor")
	await _look_at("team_human_armoury", 140, 10, "armoury")
	await _look_at("team_human_spawn", 160, 30, "spawn")
	# the bot: look along +x at it from 120 units
	var bots: PackedVector3Array = game.find_entities("player")
	print("sun: players ", bots.size())
	for b in bots:
		if game.my_entity() >= 0:
			var me: Vector4 = game.view_pos()
			if absf(b.x * 32 - me.x) < 1 and absf(-b.z * 32 - me.y) < 1:
				continue
		game.exec("setviewpos %d %d %d 180" % [b.x * 32 + 110, -b.z * 32, b.y * 32 + 10])
		await _wait(1.2)
		await _shot("bot")
		break
	# the player model (third person) at the armoury
	await _look_at("team_human_armoury", 200, 10, "rifle_view")
	game.exec("cg_thirdPerson 1")
	await _wait(1.0)
	await _shot("player_3p")
	game.exec("cg_thirdPerson 0")
	game.gfx("enhanced")
	await _wait(0.5)
	print("sun: enhanced sun visible ", _suns())
	get_tree().quit(0)
