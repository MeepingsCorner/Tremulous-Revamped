extends Node
## The microphone path alone: AudioStreamMicrophone on a bus with an
## AudioEffectCapture (voipsnd.rs's layout), frames and RMS for 3 s.
## godot --path godot res://tests/mic_probe.tscn
func _ready() -> void:
	var a := AudioServer
	print("probe: driver ", a.get_driver_name() if a.has_method("get_driver_name") else "?", " mix_rate ", a.get_mix_rate(), " inputs ", a.get_input_device_list(), " input ", a.input_device)
	var bus := a.bus_count
	a.add_bus()
	a.set_bus_name(bus, "VoipCapture")
	a.set_bus_send(bus, "Master")
	a.set_bus_volume_db(bus, float(OS.get_environment("PROBE_DB")) if OS.get_environment("PROBE_DB") != "" else -80.0)
	var fx := AudioEffectCapture.new()
	fx.buffer_length = 1.0
	a.add_bus_effect(bus, fx)
	var p := AudioStreamPlayer.new()
	p.stream = AudioStreamMicrophone.new()
	p.bus = "VoipCapture"
	add_child(p)
	p.play()
	var total := 0
	var sum := 0.0
	var peak := 0.0
	for i in 30:
		await get_tree().create_timer(0.1).timeout
		var n := fx.get_frames_available()
		if n > 0:
			var buf := fx.get_buffer(n)
			for f in buf:
				var x := (f.x + f.y) * 0.5
				sum += x * x
				peak = max(peak, abs(x))
			total += n
		if i % 10 == 9:
			print("probe: t=%.1fs frames=%d rms=%.6f peak=%.6f playing=%s pushed=%d discarded=%d" % [(i + 1) * 0.1, total, sqrt(sum / max(total, 1)), peak, p.playing, fx.get_pushed_frames() if fx.has_method("get_pushed_frames") else -1, fx.get_discarded_frames() if fx.has_method("get_discarded_frames") else -1])
	get_tree().quit(0)
