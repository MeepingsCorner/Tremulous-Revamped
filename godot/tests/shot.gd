extends Node
## Renders screenshots of a live match: xvfb-run godot --rendering-driver opengl3 --path godot res://tests/shot.tscn
var main: Node
var game
var t := 0.0
var shots := [[16.0, "user://shot_fp.png", false], [17.0, "user://shot_tp.png", true], [18.0, "user://shot_overview.png", false], [18.5, "user://shot_hbase.png", false]]
var joined := false

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")

func _process(delta: float) -> void:
	t += delta
	if not joined and t > 0.3:
		game.command("team humans")
		game.command("class rifle")
		main.get_node("HUD").close_menu()
		Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
		joined = true
	if shots.size() > 0 and t > shots[0][0]:
		var s = shots.pop_front()
		if s[1].ends_with("hbase.png"):
			var cam2 := Camera3D.new()
			add_child(cam2)
			cam2.look_at_from_position(Vector3(26, 6, 6), Vector3(38, 0.5, -2))
			cam2.current = true
		if s[1].ends_with("overview.png"):
			var cam := Camera3D.new()
			add_child(cam)
			var ceil = game.get_node_or_null("brushes_ceiling")
			if ceil:
				ceil.visible = false
			cam.look_at_from_position(Vector3(-46, 30, 30), Vector3(-10, 0, 0))
			cam.current = true
		game.third_person = s[2]
		await RenderingServer.frame_post_draw
		var img := get_viewport().get_texture().get_image()
		img.save_png(s[1])
		print("shot: saved ", ProjectSettings.globalize_path(s[1]))
	if shots.is_empty() and t > 19.5:
		get_tree().quit()
