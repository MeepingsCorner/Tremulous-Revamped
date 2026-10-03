extends Node
## Diagnostic: stand near the telenode and the egg pod and save screenshots
## into res://shots/ (the project folder) so the spawner models can be checked.
var main: Node
var game
var logf: FileAccess

func say(msg: String) -> void:
	print(msg)
	if logf:
		logf.store_line(msg)
		logf.flush()

func _ready() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path("res://shots"))
	logf = FileAccess.open(ProjectSettings.globalize_path("res://shots/log.txt"), FileAccess.WRITE)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	_run()

func _cmd(c: String) -> String:
	var r: String = game.command(c)
	say("spawner_shot: %s -> %s" % [c, "ok" if r == "" else r])
	return r

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _shot(nm: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	var p := ProjectSettings.globalize_path("res://shots/" + nm + ".png")
	get_viewport().get_texture().get_image().save_png(p)
	say("spawner_shot: saved " + p)

func _look_at(classname: String, n: int, back: float, up: float) -> bool:
	var v: PackedVector3Array = game.entity_view(classname, n)
	say("spawner_shot: entity_view %s %d -> %s" % [classname, n, str(v)])
	if v.size() != 2:
		return false
	var p := v[0]
	var a := v[1]
	var yaw := rad_to_deg(atan2(-(a.z - p.z), a.x - p.x))
	var d := Vector2(a.x - p.x, -(a.z - p.z)).normalized()
	_cmd("setviewpos %d %d %d %d" % [p.x * 32 - d.x * back, -p.z * 32 - d.y * back, p.y * 32 - 24 + up, yaw])
	return true

func _run() -> void:
	while game.is_loading():
		await _wait(0.5)
	game.set_cheats(true)
	await _wait(0.5)
	_cmd("team humans")
	_cmd("class rifle")
	main.get_node("HUD").close_menu()
	var t := 0.0
	while t < 40.0:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			break
		await _wait(0.25)
		t += 0.25
	await _wait(1.0)
	say("spawner_shot: telenodes=%d eggpods=%d reactors=%d overminds=%d" % [game.find_entities("telenode").size(), game.find_entities("eggpod").size(), game.find_entities("reactor").size(), game.find_entities("overmind").size()])
	for line in game.actor_info():
		say("actor: " + line)
	for cls in ["telenode", "eggpod"]:
		var ents: PackedVector3Array = game.find_entities(cls)
		for n in range(min(ents.size(), 2)):
			var e := ents[n]
			var q := Vector3(e.x * 32.0, -e.z * 32.0, e.y * 32.0)
			var k := 0
			for yaw in [0, 90, 180, 270]:
				var d := Vector2(cos(deg_to_rad(yaw)), sin(deg_to_rad(yaw)))
				_cmd("setviewpos %d %d %d %d" % [q.x - d.x * 110.0, q.y - d.y * 110.0, q.z + 10.0, yaw])
				await _wait(0.6)
				await _shot("%s_%d_%d" % [cls, n, k])
				k += 1
	get_tree().quit()
