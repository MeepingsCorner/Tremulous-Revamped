extends Node
## Diagnostic: r_fullscreen / r_width / r_height through the video cvars
## (the main menu path), logging the window and render size; restores the
## player's settings at the end. Log: res://shots/video_log.txt
var main: Node
var game
var logf: FileAccess

func say(msg: String) -> void:
	print(msg)
	if logf:
		logf.store_line(msg)
		logf.flush()

func _ready() -> void:
	OS.set_environment("TREM_WINDOW_CVARS", "1")
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path("res://shots"))
	logf = FileAccess.open(ProjectSettings.globalize_path("res://shots/video_log.txt"), FileAccess.WRITE)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	_run()

func _wait(s: float) -> void:
	await get_tree().create_timer(s).timeout

func _state(tag: String) -> void:
	var w := get_window()
	var vp := get_viewport().get_visible_rect().size
	say("video_test: %s mode=%d window=%s screen=%s render=%s scale_mode=%d scale_size=%s" % [tag, DisplayServer.window_get_mode(), str(DisplayServer.window_get_size()), str(DisplayServer.screen_get_size()), str(vp), w.content_scale_mode, str(w.content_scale_size)])

func _shot(nm: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png(ProjectSettings.globalize_path("res://shots/video_%s.png" % nm))

func _con(cmd: String) -> void:
	say("video_test: > %s -> '%s'" % [cmd, game.exec(cmd.replace("; ", "\n") + "\n")])

func _run() -> void:
	while game.is_loading():
		await _wait(0.5)
	await _wait(2.0)
	var keep := {}
	for n in ["r_fullscreen", "r_width", "r_height", "r_mode"]:
		keep[n] = game.exec(n + "\n")
		say("video_test: was %s" % keep[n])
	_state("start")
	_con("r_availableModes")
	_con("set r_width 1920; set r_height 1080; set r_fullscreen 1; vid_restart")
	await _wait(2.0)
	_state("1920x1080 full")
	await _shot("1920x1080_full")
	_con("set r_width 800; set r_height 600; vid_restart")
	await _wait(2.0)
	_state("800x600 full")
	await _shot("800x600_full")
	_con("set r_fullscreen 0; set r_width 1280; set r_height 720; vid_restart")
	await _wait(2.0)
	_state("1280x720 windowed")
	_con("set r_width 3840; set r_height 2160; vid_restart")
	await _wait(2.0)
	_state("3840x2160 windowed")
	await _shot("3840x2160_windowed")
	_con("set r_fullscreen 0; set r_width 0; set r_height 0; vid_restart")
	await _wait(1.0)
	_state("restored")
	get_tree().quit()
