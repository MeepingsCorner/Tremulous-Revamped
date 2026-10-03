extends Node
## Exports the arena to a pk3, loads it back through the BSP path and renders it:
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/bsp_shot.tscn
var main: Node
var game
var t := 0.0
var done := false

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	DirAccess.make_dir_recursive_absolute("user://base")
	var pk3 := ProjectSettings.globalize_path("user://base/zz-arena.pk3")
	print("bsp: export -> '", game.export_arena_pk3(pk3), "'")
	var paths := PackedStringArray([pk3])
	print("bsp: maps = ", game.list_maps(paths))
	print("bsp: load -> '", game.load_bsp(paths, "arena"), "'")
	main.get_node("HUD").close_menu()
	game.command("team humans")
	game.command("class rifle")
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE

func _process(delta: float) -> void:
	t += delta
	if not done and t > 15.0:
		done = true
		var h: Dictionary = game.get_hud()
		print("bsp: alive=", h.alive, " team=", h.team, " weapon=", h.weapon)
		await RenderingServer.frame_post_draw
		get_viewport().get_texture().get_image().save_png("user://shot_bsp.png")
		print("bsp: ", "PASS" if h.alive else "FAIL")
		get_tree().quit(0 if h.alive else 1)
