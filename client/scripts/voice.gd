extends Node3D
## 20 ms Opus frames; bounded jitter queues, spatial Godot playback, opt-in capture.
signal status(message: String)
var network: Node
var codec: RefCounted
var capture: AudioEffectCapture
var microphone: AudioStreamPlayer
var streams: Dictionary = {}
var sequence := 0
var microphone_enabled := false
var bitrate := 24000
var bus_index := -1
var radio_target := ""
var radio_codec: RefCounted
var radio_suppressed: Dictionary = {}
var playback_gain := 1.0

func prefer_radio(user: String) -> void:
	radio_suppressed[user] = Time.get_ticks_msec() + 300
	if streams.has(user):
		streams[user].player.volume_linear = 0

func attach(transport: Node) -> void:
	network = transport
	network.negotiated.connect(_configure)
	network.voice_packet.connect(_receive)
	bus_index = AudioServer.bus_count
	AudioServer.add_bus()
	AudioServer.set_bus_name(bus_index, "SSC microphone")
	AudioServer.set_bus_mute(bus_index, true)
	capture = AudioEffectCapture.new()
	capture.buffer_length = 0.2
	AudioServer.add_bus_effect(bus_index, capture)
	microphone = AudioStreamPlayer.new()
	microphone.stream = AudioStreamMicrophone.new()
	microphone.bus = "SSC microphone"
	add_child(microphone)

func _configure(server_bitrate: int) -> void:
	clear()
	bitrate = server_bitrate
	if not ClassDB.class_exists("OpusCodec"):
		status.emit("Opus integration unavailable; use the Nix-packaged client")
		return
	codec = ClassDB.instantiate("OpusCodec")
	configure_radio(8000)
	if not codec.configure(bitrate):
		codec = null
		status.emit("Could not configure Opus")

func configure_radio(server_bitrate: int) -> void:
	if ClassDB.class_exists("OpusCodec"):
		radio_codec = ClassDB.instantiate("OpusCodec")
		if not radio_codec.configure(server_bitrate):
			radio_codec = null

func set_microphone(enabled: bool) -> void:
	microphone_enabled = enabled and codec != null and network.joined
	capture.clear_buffer()
	if microphone_enabled:
		microphone.play()
	else:
		microphone.stop()
	status.emit("Microphone on" if microphone_enabled else "Microphone muted")

func _receive(user: String, serial: int, _timestamp: int, packet: PackedByteArray, route: int, epoch: int = 0) -> void:
	if not codec or user == network.user:
		return
	if streams.has(user):
		var previous_epoch := int(streams[user].get("epoch",0))
		if epoch < previous_epoch:
			return
		if epoch > previous_epoch:
			streams[user].player.stop()
			streams[user].player.queue_free()
			streams.erase(user)
	if not streams.has(user):
		var decoder = ClassDB.instantiate("OpusCodec")
		if not decoder.configure(bitrate):
			return
		var player := AudioStreamPlayer3D.new()
		var stream := AudioStreamGenerator.new()
		stream.mix_rate = 48000
		stream.buffer_length = 0.15
		player.stream = stream
		player.max_distance = 30
		player.unit_size = 2
		add_child(player)
		player.play()
		streams[user] = {"decoder":decoder,"player":player,"queue":[],"sequence":-1,"started":false,"elapsed":0.0,"loss":0,"epoch":epoch}
	var state: Dictionary = streams[user]
	serial = serial & 0xffffffff
	if int(state.sequence) >= 0:
		var advance := (serial - int(state.sequence)) & 0xffffffff
		if advance == 0 or advance >= 0x80000000:
			return
	state.sequence = serial
	state.route = route
	state.player.panning_strength = 0.0 if route == 1 else 1.0
	state.player.max_distance = 0.0 if route == 1 else 30.0
	state.player.volume_linear = 0.0 if Time.get_ticks_msec() < int(radio_suppressed.get(user,0)) else playback_gain
	state.player.attenuation_model = AudioStreamPlayer3D.ATTENUATION_DISABLED if route == 1 else AudioStreamPlayer3D.ATTENUATION_INVERSE_DISTANCE
	var queue: Array = state.queue
	queue.append(packet)
	while queue.size() > 6:
		queue.pop_front()
	if queue.size() >= 3:
		state.started = true

func positions(players: Array) -> void:
	var present: Dictionary = {}
	for player in players:
		var user := str(player.id)
		present[user] = true
		if streams.has(user):
			var p: Array = player.pose
			streams[user].player.global_position = Vector3(float(p[0]),float(p[1]),float(p[2]))
	for user in streams.keys():
		if not present.has(user) and int(streams[user].get("route", 0)) == 0:
			streams[user].player.stop()
			streams[user].player.queue_free()
			streams.erase(user)

func call_state(calls: Array) -> void:
	var members: Dictionary = {}
	for call in calls:
		for user in call.get("participants", []):
			members[str(user)] = true
	for user in streams.keys():
		if int(streams[user].get("route", 0)) == 1 and not members.has(user):
			streams[user].player.stop()
			streams[user].player.queue_free()
			streams.erase(user)

func _process(delta: float) -> void:
	for user in radio_suppressed.keys():
		if Time.get_ticks_msec() >= int(radio_suppressed[user]):
			radio_suppressed.erase(user)
	for user in streams:
		streams[user].player.volume_linear = 0.0 if radio_suppressed.has(user) else playback_gain
	if not network or not network.joined:
		if microphone_enabled:
			set_microphone(false)
		return
	if microphone_enabled and codec:
		if capture.get_frames_available() > 960 * 6:
			capture.clear_buffer()
		for i in 4:
			if capture.get_frames_available() < 960:
				break
			var pcm := capture.get_buffer(960)
			sequence += 1
			if not radio_target.is_empty() and radio_codec:
				network.radio_voice(radio_target, radio_codec.encode(pcm))
			else:
				network.voice(sequence, Time.get_ticks_msec(), codec.encode(pcm))
	for state in streams.values():
		if not state.started:
			continue
		state.elapsed += delta
		if float(state.elapsed) < 0.02:
			continue
		state.elapsed = fmod(float(state.elapsed), 0.02)
		var queue: Array = state.queue
		var packet: PackedByteArray = queue.pop_front() if not queue.is_empty() else PackedByteArray()
		state.loss = int(state.loss) + 1 if packet.is_empty() else 0
		if int(state.loss) > 3:
			state.started = false
			continue
		var pcm: PackedVector2Array = state.decoder.decode(packet)
		var playback: AudioStreamGeneratorPlayback = state.player.get_stream_playback()
		if playback and playback.get_frames_available() >= pcm.size():
			playback.push_buffer(pcm)

func clear() -> void:
	if microphone:
		set_microphone(false)
	for state in streams.values():
		state.player.stop()
		state.player.queue_free()
	streams.clear()
	codec = null
	radio_codec = null
	radio_target = ""
	radio_suppressed.clear()

func _exit_tree() -> void:
	clear()
	if bus_index >= 0:
		AudioServer.remove_bus(bus_index)
