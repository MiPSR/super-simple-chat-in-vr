extends Node
## Named source zones select ordinary Godot buses; room size never sets wetness.
var zones: Array[Dictionary] = []
var bus_names: Array[StringName] = []

func configure(specs: Array, ceiling: float) -> void:
	clear()
	for spec in specs:
		var name := StringName("SSC zone " + str(get_instance_id()) + " " + str(spec.id))
		var index := AudioServer.bus_count
		AudioServer.add_bus()
		AudioServer.set_bus_name(index, name)
		AudioServer.set_bus_send(index, &"Master")
		bus_names.append(name)
		var effects: Dictionary = spec.get("effects", {})
		var reverb_amount := clampf(float(effects.get("reverb", 0)), 0, ceiling)
		if reverb_amount > 0:
			var reverb := AudioEffectReverb.new()
			reverb.wet = reverb_amount
			reverb.dry = 1.0 - reverb_amount
			AudioServer.add_bus_effect(index, reverb)
		var echo_amount := clampf(float(effects.get("echo", 0)), 0, ceiling)
		if echo_amount > 0:
			var delay := AudioEffectDelay.new()
			delay.dry = 1.0 - echo_amount * 0.5
			delay.set_tap1_active(true)
			delay.set_tap1_delay_ms(180)
			delay.set_tap1_level_db(linear_to_db(echo_amount))
			delay.set_feedback_active(true)
			delay.set_feedback_delay_ms(180)
			delay.set_feedback_level_db(linear_to_db(echo_amount * 0.6))
			AudioServer.add_bus_effect(index, delay)
		var distortion_amount := clampf(float(effects.get("distortion", 0)), 0, ceiling)
		if distortion_amount > 0:
			var distortion := AudioEffectDistortion.new()
			distortion.drive = distortion_amount
			distortion.post_gain = -6.0 * distortion_amount
			AudioServer.add_bus_effect(index, distortion)
		if bool(effects.get("compression", false)):
			AudioServer.add_bus_effect(index, AudioEffectCompressor.new())
		# A final limiter bounds amplified/effect feedback output.
		AudioServer.add_bus_effect(index, AudioEffectHardLimiter.new())
		var p: Array = spec.position
		var s: Array = spec.size
		var size := Vector3(float(s[0]), float(s[1]), float(s[2]))
		var center := Vector3(float(p[0]), float(p[1]), float(p[2]))
		zones.append({"bounds":AABB(center - size / 2.0, size), "bus":name})

func route(position: Vector3, dry: bool = false) -> StringName:
	if not dry:
		for zone in zones:
			if zone.bounds.has_point(position):
				return zone.bus
	return &"Master"

func clear() -> void:
	for name in bus_names:
		var index := AudioServer.get_bus_index(name)
		if index >= 0:
			AudioServer.remove_bus(index)
	bus_names.clear()
	zones.clear()

func _exit_tree() -> void:
	clear()
