extends Node
## Particle systems from the original scripts, spawned in front of the
## spectator camera and screenshotted.
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/fx_shot.tscn -- --map=nexus6
var main: Node
var game
var t := 0.0
var map := "nexus6"
var step := 0
var systems := [
	["impacts", ["models/weapons/rifle/impactPS", "models/weapons/grenade/impactPS", "models/weapons/lcannon/impactPS", "models/weapons/mdriver/impactPS"]],
	["buildables", ["humanBuildableDestroyedPS", "alienBuildableDestroyedPS", "humanBuildableDamagedPS", "alienEvolvePS"]],
	["misc", ["models/weapons/flamer/muzzlePS", "jetPackHoverPS", "alienAcidTubePS", "humanBleedPS"]],
]

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--map="):
			map = a.substr(6)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	print("fx: load ", map, " -> '", game.load_bsp(game.pk3_paths(), map), "'")
	main.get_node("HUD").close_menu()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE

func _process(delta: float) -> void:
	t += delta
	if step < systems.size() and t > 1.5 + step * 2.0:
		var cam: Camera3D = game.view_camera()
		var f := -cam.global_transform.basis.z
		var r := cam.global_transform.basis.x
		var n := 0
		for s in systems[step][1]:
			var p: Vector3 = cam.global_position + f * 4.0 + r * (float(n) - 1.5) * 1.5
			print("fx: ", s, " -> ", game.spawn_fx_named(s, p))
			n += 1
		step += 1
		await get_tree().create_timer(0.35).timeout
		await RenderingServer.frame_post_draw
		get_viewport().get_texture().get_image().save_png("user://fx_%s_%s.png" % [map, systems[step - 1][0]])
		print("fx: saved ", systems[step - 1][0])
	elif step >= systems.size() and t > 2.0 + step * 2.0:
		get_tree().quit()
