extends Node
## A second client: connects to --addr=host:port and shoots the screen once Active (team picker).
## godot --path godot res://tests/host_join_probe.tscn -- --addr=127.0.0.1:30777
var main: Node
var game
var t := 0.0
var active_at := -1.0
var addr := "127.0.0.1:30777"
var shot := false
var last_log := 0.0

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--addr="):
			addr = a.substr(7)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.bots_per_team = 0
	print("host_join: ", game.exec("set cl_allowDownload 0\nset name Joiner\nconnect " + addr))

func _process(delta: float) -> void:
	t += delta
	if t - last_log > 5.0:
		last_log = t
		print("host_join t=%.0f %s" % [t, String(game.net_status()).substr(0, 90)])
	if active_at < 0.0 and String(game.net_status()).begins_with("cl=Active"):
		active_at = t
		print("host_join: ACTIVE at %.1f s" % t)
	if active_at >= 0.0 and not shot and t - active_at > 6.0:
		shot = true
		await RenderingServer.frame_post_draw
		var img := get_viewport().get_texture().get_image()
		img.save_png("user://joiner_team_picker.png")
		print("host_join: shot ", ProjectSettings.globalize_path("user://joiner_team_picker.png"))
	if (shot and t - active_at > 9.0) or t > 150.0:
		print("host_join: end t=%.0f active=%s" % [t, str(active_at >= 0.0)])
		get_tree().quit(0 if active_at >= 0.0 else 1)
