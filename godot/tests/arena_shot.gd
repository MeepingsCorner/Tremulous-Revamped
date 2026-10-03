extends Node
## Arena with original models: xvfb-run godot --rendering-driver opengl3 --path godot res://tests/arena_shot.tscn
var main: Node
var game
var t := 0.0
var step := 0

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.load_arena()
	main.get_node("HUD").close_menu()
	game.command("team humans")
	game.command("class rifle")
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE

func _shot(name: String) -> void:
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png("user://" + name)
	print("arena: saved ", name)

func _process(delta: float) -> void:
	t += delta
	if step == 0 and t > 6.0:
		step = 1
		var cam := Camera3D.new()
		add_child(cam)
		cam.look_at_from_position(Vector3(26, 6, 6), Vector3(38, 0.5, -2))
		cam.current = true
		_shot("arena_hbase.png")
	elif step == 1 and t > 6.5:
		step = 2
		var cam := Camera3D.new()
		add_child(cam)
		cam.look_at_from_position(Vector3(-28, 5, -6), Vector3(-38, 0.5, 2))
		cam.current = true
		_shot("arena_abase.png")
	elif step == 2 and t > 7.0:
		get_tree().quit()
