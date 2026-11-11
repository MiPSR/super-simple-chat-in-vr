extends SceneTree
const Network = preload("res://scripts/network.gd")
const VideoSurface = preload("res://scripts/video_surface.gd")
const CreatorRuntime = preload("res://scripts/creator_runtime.gd")
var network: Node
var commands := ""
var serial := 0
var emit_pose := true
var tick_count := 0
var snapshot_count := 0
var started := 0
var codec: RefCounted
var raised_hands := false
var radio_codec: RefCounted
var video: Node3D
var media_sampled := -1.0
var stopping := false

func _stop(code: int = 0) -> void:
	if stopping:
		return
	stopping = true
	emit_pose = false
	network.close()
	if video:
		video.close()
		video.queue_free()
		video = null
	codec = null
	radio_codec = null
	# Let the mixer/main thread retire stopped generator playbacks before exit.
	await create_timer(0.1).timeout
	await process_frame
	quit(code)

func _update_video(data: Dictionary) -> void:
	var source := OS.get_environment("SSC_TEST_PUBLIC_VIDEO")
	if source.is_empty():
		return
	var state: Dictionary = data.get("media", {}).get("screen", {})
	if state.is_empty() or float(state.get("sampled_at", 0)) < media_sampled:
		return
	media_sampled = float(state.get("sampled_at", 0))
	if not video and str(state.get("source", "")) == source:
		video = VideoSurface.new()
		root.add_child(video)
		video.failed.connect(func(message: String):
			print("SSCTEST " + JSON.stringify({"kind":251,"data":{"error":message}}))
		)
		video.setup({"position":[0,2,-3],"size":[3.2,1.8]})
	if video:
		video.update_timeline(network.media_timeline(state))

func _initialize() -> void:
	var file := OS.get_environment("SSC_TEST_CONFIG")
	var config: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(file))
	commands = str(config.commands)
	network = Network.new()
	root.add_child.call_deferred(network)
	network.message.connect(func(kind: int, data: Dictionary):
		if kind == 3:
			tick_count += 1
		if kind == 2:
			snapshot_count += 1
		if kind in [2, 3]:
			_update_video(data.get("snapshot", data))
		print("SSCTEST " + JSON.stringify({"kind":kind,"data":data}))
		if kind == 24:
			assert(radio_codec.decode(str(data.data).hex_decode()).size() == 960)
	)
	network.status.connect(func(text: String): print("SSCTEST " + JSON.stringify({"status":text})))
	network.negotiated.connect(func(bitrate: int):
		codec = ClassDB.instantiate("OpusCodec")
		assert(codec.configure(bitrate))
		radio_codec = ClassDB.instantiate("OpusCodec")
		assert(radio_codec.configure(8000))
	)
	network.voice_packet.connect(func(user: String, sequence: int, _time: int, packet: PackedByteArray, route: int, epoch: int):
		assert(codec.decode(packet).size() == 960)
		print("SSCTEST " + JSON.stringify({"kind":20,"data":{"user":user,"sequence":sequence,"bytes":packet.size(),"route":route,"epoch":str(epoch)}}))
	)
	network.connect_grant(config.grant)
	started = Time.get_ticks_msec()

func _process(delta: float) -> bool:
	if stopping:
		return false
	if emit_pose:
		var poses: Array[Transform3D] = [Transform3D(Basis.IDENTITY, Vector3(0,1.65,0)), Transform3D(Basis.IDENTITY,Vector3(-0.05 if raised_hands else -0.3,1.7 if raised_hands else 1,0)), Transform3D(Basis.IDENTITY,Vector3(0.05 if raised_hands else 0.3,1.7 if raised_hands else 1,0))]
		network.pose(delta, poses)
	for _index in 32:
		var path := commands + "." + str(serial + 1)
		if not FileAccess.file_exists(path):
			break
		var command = JSON.parse_string(FileAccess.get_file_as_string(path))
		if command is Dictionary and int(command.serial) > serial:
			serial = int(command.serial)
			match str(command.action):
				"command":
					network.command(int(command.kind), command.data)
				"creator_media":
					var creator := CreatorRuntime.new()
					root.add_child(creator)
					creator.set_process(false)
					var scope := Node3D.new()
					root.add_child(scope)
					scope.set_meta("ssc_network", network)
					creator.scope = scope
					creator.script_error.connect(func(message: String): push_error(message))
					var vm = ClassDB.instantiate("CreatorVm")
					assert(vm.load_script(str(command.source), PackedStringArray(["media"])).is_empty())
					creator.scripts.append({"vm":vm,"attachment":"world","failed":false})
					creator.dispatch("action", {})
					creator.free()
					scope.free()
				"pause_pose":
					emit_pose = false
				"video_probe":
					var result := {"serial":serial,"ready":false}
					if video and video.decoder and not video.source.is_empty():
						var expected: float = video.target + (Time.get_ticks_msec()/1000.0 - video.received)*video.rate if video.playing else video.target
						result.merge({"ready":not video.broken,"position":video.decoder.position(),"expected":expected,"frames":video.decoded_frames,"pcm":video.mixed_frames,"playing":video.playing}, true)
					print("SSCTEST " + JSON.stringify({"kind":250,"data":result}))
				"raise_hands":
					raised_hands = true
				"voice":
					var pcm := PackedVector2Array()
					pcm.resize(960)
					pcm.fill(Vector2(0.1,0.1))
					network.voice(int(command.sequence), Time.get_ticks_msec(), codec.encode(pcm))
				"radio_voice":
					var pcm := PackedVector2Array()
					pcm.resize(960)
					pcm.fill(Vector2(0.1,0.1))
					network.radio_voice(str(command.target),radio_codec.encode(pcm))
				"malformed":
					if network.joined:
						network.peer.send(1, PackedByteArray([1, 2, 3]), 0)
						var packet: PackedByteArray = network.cipher.seal(0, 0, 4, '{"text":"must-not-arrive"}'.to_utf8_buffer())
						packet[30] ^= 1
						network.peer.send(0, packet, ENetPacketPeer.FLAG_RELIABLE)
				"quit":
					_stop()
			print("SSCTEST " + JSON.stringify({"kind":254,"data":{"serial":serial}}))
	if Time.get_ticks_msec() - started > (90000 if not OS.get_environment("SSC_TEST_PUBLIC_VIDEO").is_empty() else 45000):
		_stop(2)
	return false
