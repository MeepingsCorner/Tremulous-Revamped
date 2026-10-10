extends Node
## The port's base lights (the reactor's blue, the overmind's green
## OmniLight3D) in the classic (vanilla) and enhanced modes on ATCS, and
## the reactor idle and under attack in vanilla (user://reactor_*.png).
## godot --path godot res://tests/reactor_check.tscn
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

func _shot(kind: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	var path := "user://reactor_%s.png" % kind
	get_viewport().get_texture().get_image().save_png(path)
	print("reactor: %s -> %s" % [kind, ProjectSettings.globalize_path(path)])

## Visible base lights: OmniLight3D children of TremGame outside the dlight group.
func _base_lights() -> String:
	var out := []
	for c in game.get_children():
		if c is OmniLight3D and not c.is_in_group("trem_dlight") and c.is_visible_in_tree():
			out.append("%s" % c.light_color)
	var all := 0
	for c in game.get_children():
		if c is OmniLight3D and not c.is_in_group("trem_dlight"):
			all += 1
	return "%d of %d %s" % [out.size(), all, out]

func _run() -> void:
	await _wait(1.0)
	game.exec("set net_enabled 1\nset net_port 30798\nset name Checker\ndevmap atcs")
	while not (game.has_world() and not game.is_loading()):
		await _wait(0.5)
	hud.close_menu()
	await _wait(2.0)
	game.set_cheats(true)
	game.exec("team humans")
	game.exec("class rifle")
	await _wait(3.0)
	hud.close_menu()
	for mode in ["vanilla", "enhanced", "vanilla"]:
		game.gfx(mode)
		await _wait(0.5)
		print("reactor: %s base lights visible: %s" % [mode, _base_lights()])
	var r: PackedVector3Array = game.find_entities("team_human_reactor")
	var a := r[0]
	game.exec("setviewpos %d %d %d 180" % [a.x * 32 + 160, -a.z * 32, a.y * 32 + 20])
	await _wait(1.5)
	await _shot("vanilla_idle")
	var h: PackedStringArray = game.structure_health("team_human_reactor")
	var parts := h[0].split(" ")
	game.test_set_health(int(parts[0]), int(parts[1]) - 100)
	await _wait(0.3)
	print("reactor: under attack, base lights visible: ", _base_lights())
	await _shot("vanilla_attacked")
	get_tree().quit(0)
