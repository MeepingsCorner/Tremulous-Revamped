extends Node
## The microphone inside a running game (FMOD on): the game's VoipCapture
## bus and a second, independent microphone bus, frames and RMS.
## godot --path godot res://tests/mic_ingame.tscn
var main: Node
var game

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.bots_per_team = 0
	_run()

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _rms(fx: AudioEffectCapture) -> String:
	var n := fx.get_frames_available()
	var sum := 0.0
	var peak := 0.0
	if n > 0:
		for f in fx.get_buffer(n):
			var x := (f.x + f.y) * 0.5
			sum += x * x
			peak = max(peak, abs(x))
	return "frames=%d rms=%.6f peak=%.6f pushed=%d" % [n, sqrt(sum / max(n, 1)), peak, fx.get_pushed_frames()]

func _run() -> void:
	await _wait(1.0)
	game.exec("set net_enabled 1\nset net_port 30797\nset name Talker\ndevmap atcs")
	while not (game.has_world() and not game.is_loading()):
		await _wait(0.5)
	await _wait(2.0)
	game.set_cheats(true)
	game.exec("team humans")
	game.exec("class rifle")
	await _wait(3.0)
	print("mic: toggle ", game.exec("voip_toggle"))
	await _wait(1.0)
	var a := AudioServer
	print("mic: buses ", a.bus_count, " input ", a.input_device, " mix ", a.get_mix_rate())
	for b in a.bus_count:
		print("mic: bus %d %s vol=%.1f mute=%s effects=%d send=%s" % [b, a.get_bus_name(b), a.get_bus_volume_db(b), a.is_bus_mute(b), a.get_bus_effect_count(b), a.get_bus_send(b)])
	var vb := a.get_bus_index("VoipCapture")
	var mics := main.find_children("VoipMicrophone", "", true, false)
	for m in game.find_children("*", "AudioStreamPlayer", true, false):
		if m.stream is AudioStreamMicrophone:
			print("mic: player ", m.get_path(), " playing=", m.playing, " bus=", m.bus, " vol=", m.volume_db, " paused=", m.stream_paused, " process_mode=", m.process_mode)
	# a second microphone on its own bus, while the game runs
	var bus := a.bus_count
	a.add_bus()
	a.set_bus_name(bus, "ProbeMic")
	a.set_bus_volume_db(bus, -80.0)
	var fx := AudioEffectCapture.new()
	fx.buffer_length = 1.0
	a.add_bus_effect(bus, fx)
	var p := AudioStreamPlayer.new()
	p.stream = AudioStreamMicrophone.new()
	p.bus = "ProbeMic"
	add_child(p)
	p.play()
	for i in 4:
		await _wait(0.5)
		var gfx: AudioEffectCapture = a.get_bus_effect(vb, 0) if vb >= 0 else null
		print("mic: t=%.1f probe %s | game pushed=%s | %s" % [(i + 1) * 0.5, _rms(fx), str(gfx.get_pushed_frames()) if gfx else "-", game.voip_hud_state().get_slice(" hud", 0)])
	get_tree().quit(0)
