extends Node3D
## Protected SSTV/FT8-style payloads, not external modem interoperability.
signal status(message: String)
const Voice = preload("res://scripts/voice.gd")
var transport: Node
var receiver: Node3D
var devices: Dictionary = {}
var bus := StringName()
var noise := 0.15
var interference := 0.35
var policy_filter := 1500.0
var policy_noise := 0.15
var preset := "standard"
var allowed_presets: Array = ["standard", "narrow", "weak"]
var actor := ""
var digital_last: Dictionary = {}
var digital_intervals := {"ft8":2000,"sstv":30000}

func bind_actor(user: String) -> void:
	if actor != user:
		actor = user
		digital_last.clear()

func apply_preset(name: String) -> bool:
	if name not in ["standard", "narrow", "weak"] or not allowed_presets.has(name):
		return false
	preset = name
	var factor := 1.0 if name == "standard" else 0.75 if name == "narrow" else 0.5
	noise = clampf(policy_noise + (0.0 if name == "standard" else 0.1 if name == "narrow" else 0.3), 0, 0.8)
	var index := AudioServer.get_bus_index(bus)
	if index >= 0:
		AudioServer.get_bus_effect(index, 0).cutoff_hz = maxf(300, policy_filter * factor)
	for device in devices.values():
		device.noise = maxf(noise, _preset_noise(str(device.preset)))
	return true

func _preset_noise(name: String) -> float:
	return clampf(policy_noise + (0.0 if name == "standard" else 0.1 if name == "narrow" else 0.3), 0, 0.8)

func transmit(id: String, mode: String, data: String) -> bool:
	if not transport or not transport.joined or not devices.has(id):
		return false
	if not digital_intervals.has(mode):
		return false
	var now := Time.get_ticks_msec()
	if digital_last.has(mode) and now - int(digital_last[mode]) < int(digital_intervals[mode]):
		return false
	if mode == "ft8":
		if data.is_empty() or data.to_utf8_buffer().size() > 40:
			return false
		for byte in data.to_utf8_buffer():
			if byte < 32 or byte > 126:
				return false
	elif mode == "sstv":
		if data.to_utf8_buffer().size() != 6144:
			return false
		for byte in data.to_utf8_buffer():
			if not ((byte >= 48 and byte <= 57) or (byte >= 65 and byte <= 70) or (byte >= 97 and byte <= 102)):
				return false
	else:
		return false # Scripts cannot enable microphone capture or send raw voice.
	digital_last[mode] = now
	transport.command(25, {"target":id,"mode":mode,"data":data})
	return true

func attach(network: Node) -> void:
	transport = network
	receiver = Voice.new()
	add_child(receiver)
	receiver.network = network
	receiver._configure(8000)
	bus = StringName("SSC narrowband " + str(get_instance_id()))
	var index := AudioServer.bus_count
	AudioServer.add_bus()
	AudioServer.set_bus_name(index, bus)
	var band := AudioEffectBandPassFilter.new()
	band.cutoff_hz = 1500
	band.resonance = 0.5
	AudioServer.add_bus_effect(index, band)
	var distortion := AudioEffectDistortion.new()
	distortion.drive = 0.15
	AudioServer.add_bus_effect(index, distortion)
	AudioServer.add_bus_effect(index, AudioEffectHardLimiter.new())

func configure(specs: Array, policy: Dictionary = {}) -> void:
	clear()
	policy_noise = clampf(float(policy.get("noise",0.15)),0,0.8)
	policy_filter = clampf(float(policy.get("filter_hz",1500)),300,3000)
	digital_intervals.ft8 = clampi(int(policy.get("ft8_interval",2)),1,120) * 1000
	digital_intervals.sstv = clampi(int(policy.get("sstv_interval",30)),5,600) * 1000
	allowed_presets = policy.get("presets", ["standard", "narrow", "weak"])
	if not allowed_presets.has(preset):
		preset = str(allowed_presets[0])
	interference = clampf(float(policy.get("interference",0.35)),0,0.8)
	apply_preset(preset)
	receiver._configure(8000)
	for spec in specs:
		var object_preset := str(spec.get("preset", "standard"))
		if not allowed_presets.has(object_preset):
			object_preset = str(allowed_presets[0])
		var object_bus := StringName(str(bus) + " " + str(spec.id))
		var object_index := AudioServer.bus_count
		AudioServer.add_bus()
		AudioServer.set_bus_name(object_index, object_bus)
		AudioServer.set_bus_send(object_index, bus)
		var object_filter := AudioEffectBandPassFilter.new()
		var factor := 1.0 if object_preset == "standard" else 0.75 if object_preset == "narrow" else 0.5
		object_filter.cutoff_hz = maxf(300, policy_filter * factor)
		AudioServer.add_bus_effect(object_index, object_filter)
		AudioServer.set_bus_effect_enabled(object_index, 0, object_preset != "standard")
		var device := Node3D.new()
		add_child(device)
		var p: Array = spec.position
		device.position = Vector3(float(p[0]),float(p[1]),float(p[2]))
		var body := MeshInstance3D.new()
		var box := BoxMesh.new()
		box.size = Vector3(0.6,0.4,0.3)
		body.mesh = box
		device.add_child(body)
		var image := Image.create(64,48,false,Image.FORMAT_L8)
		var texture := ImageTexture.create_from_image(image)
		var screen := MeshInstance3D.new()
		var quad := QuadMesh.new()
		quad.size = Vector2(0.48,0.36)
		screen.mesh = quad
		screen.position.z = 0.16
		var material := StandardMaterial3D.new()
		material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		material.albedo_texture = texture
		screen.material_override = material
		device.add_child(screen)
		var label := Label3D.new()
		label.text = "Radio · channel " + str(spec.channel)
		label.font_size = 28
		label.pixel_size = 0.004
		label.position.y = 0.45
		device.add_child(label)
		var tone := AudioStreamPlayer3D.new()
		var stream := AudioStreamGenerator.new()
		stream.mix_rate = 48000
		stream.buffer_length = 0.1
		tone.stream = stream
		tone.bus = object_bus
		tone.max_distance = 30
		device.add_child(tone)
		devices[str(spec.id)] = {"node":device,"texture":texture,"label":label,"tone":tone,"until":0,"mode":"voice","phase":0.0,"samples":0,"data":"","noise":maxf(noise,_preset_noise(object_preset)),"preset":object_preset,"bus":object_bus}

static func degraded_image(data: PackedByteArray, sequence: int, amount: float = 0.15) -> Image:
	if data.size() != 64 * 48:
		return null
	var pixels := data.duplicate()
	var rng := RandomNumberGenerator.new()
	rng.seed = sequence
	for i in pixels.size():
		var spread := int(clampf(amount,0,0.8) * 80)
		pixels[i] = clampi(int(pixels[i]) + rng.randi_range(-spread,spread),0,255)
		if rng.randf() < amount / 6.0:
			pixels[i] = 0
	return Image.create_from_data(64,48,false,Image.FORMAT_L8,pixels)

func receive(event: Dictionary) -> void:
	if str(event.get("sender", "")) == actor and digital_intervals.has(str(event.get("mode", ""))):
		digital_last[str(event.mode)] = Time.get_ticks_msec()
	var id := str(event.get("radio", ""))
	if not devices.has(id):
		return
	var device: Dictionary = devices[id]
	device.until = Time.get_ticks_msec() + (150 if event.mode == "voice" else 1000 if event.mode == "ft8" else 2000)
	device.mode = str(event.mode)
	device.data = str(event.data)
	device.noise = clampf(maxf(noise, _preset_noise(str(device.preset))) + (interference if bool(event.get("interference",false)) else 0.0),0,0.8)
	device.label.modulate = Color(1,0.6,0.4) if bool(event.get("interference",false)) else Color.WHITE
	if event.mode == "voice":
		var key := id + ":" + str(event.sender)
		if not receiver.streams.has(key) and receiver.streams.size() >= 32:
			return
		receiver._receive(key,int(event.sequence),0,str(event.data).hex_decode(),0)
		if receiver.streams.has(key):
			var state: Dictionary = receiver.streams[key]
			state.player.global_position = device.node.global_position
			state.player.bus = device.bus
			state.radio_seen = Time.get_ticks_msec()
	elif event.mode == "ft8":
		device.label.text = "FT8 · " + str(event.sender) + "\n" + str(event.data)
		status.emit(device.label.text)
	elif event.mode == "sstv":
		var image := degraded_image(str(event.data).hex_decode(),int(event.sequence),float(device.noise))
		if image:
			device.texture.update(image)
			device.label.text = "SSTV · " + str(event.sender)
			status.emit(device.label.text)

func _process(_delta: float) -> void:
	for device in devices.values():
		if Time.get_ticks_msec() < int(device.until):
			if not device.tone.playing:
				device.tone.play()
			var playback: AudioStreamGeneratorPlayback = device.tone.get_stream_playback()
			var count := mini(playback.get_frames_available(),960) if playback else 0
			var pcm := PackedVector2Array()
			for _sample in count:
				var value := randf_range(-1,1) * float(device.noise) * 0.025
				if device.mode != "voice" and not str(device.data).is_empty():
					var symbol := str(device.data).unicode_at((int(device.samples) / 3840) % str(device.data).length()) % 8
					device.phase = fmod(float(device.phase) + TAU * (1000 + symbol * 125) / 48000.0, TAU)
					value += sin(float(device.phase)) * 0.025
				device.samples = int(device.samples) + 1
				pcm.append(Vector2(value,value))
			if count > 0:
				playback.push_buffer(pcm)
		elif device.tone.playing:
			device.tone.stop()
	if receiver:
		for key in receiver.streams.keys():
			if Time.get_ticks_msec() - int(receiver.streams[key].get("radio_seen",0)) > 5000:
				receiver.streams[key].player.stop()
				receiver.streams[key].player.queue_free()
				receiver.streams.erase(key)

func environment(target_bus: StringName) -> void:
	var index := AudioServer.get_bus_index(bus)
	if index >= 0 and AudioServer.get_bus_send(index) != target_bus:
		AudioServer.set_bus_send(index,target_bus)

func clear() -> void:
	if receiver:
		receiver.clear()
	for device in devices.values():
		device.tone.stop()
		device.node.queue_free()
		var index := AudioServer.get_bus_index(device.bus)
		if index >= 0:
			AudioServer.remove_bus(index)
	devices.clear()

func _exit_tree() -> void:
	clear()
	var index := AudioServer.get_bus_index(bus)
	if index >= 0:
		AudioServer.remove_bus(index)
