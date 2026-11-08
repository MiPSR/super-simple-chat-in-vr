extends RefCounted
const Network = preload("res://scripts/network.gd")
const Voice = preload("res://scripts/voice.gd")

static func frame(user: String, serial: int, epoch: int, packet: PackedByteArray, route: int = 0) -> PackedByteArray:
	var id := user.to_utf8_buffer()
	var plain := PackedByteArray()
	plain.resize(23 + id.size())
	plain[0] = 20
	plain[1] = 128 | route
	plain[2] = id.size()
	for index in id.size(): plain[3+index] = id[index]
	plain.encode_u64(3+id.size(), epoch)
	plain.encode_u32(11+id.size(), serial)
	plain.encode_u64(15+id.size(), 7)
	plain.append_array(packet)
	return plain

static func run() -> void:
	var tree: SceneTree = Engine.get_main_loop()
	var baseline := AudioServer.bus_count
	var network := Network.new()
	tree.root.add_child(network)
	network.user = "listener"
	network.joined = true
	var voice := Voice.new()
	tree.root.add_child(voice)
	voice.attach(network)
	network.negotiated.emit(24000)
	var encoder = ClassDB.instantiate("OpusCodec")
	assert(encoder.configure(24000))
	var pcm := PackedVector2Array()
	pcm.resize(960)
	pcm.fill(Vector2(0.1,0.1))
	var packet: PackedByteArray = encoder.encode(pcm)
	var epoch := (1 << 53) + 10 # Binary epochs must not round through JSON floats.
	network._decode_voice(frame("speaker",500,epoch,packet))
	var old_player: AudioStreamPlayer3D = voice.streams.speaker.player
	var playbacks := [weakref(old_player.get_stream_playback())]
	network._decode_voice(frame("speaker",1,epoch+1,packet,1))
	var current: Dictionary = voice.streams.speaker
	assert(current.epoch == epoch+1 and current.sequence == 1 and current.route == 1)
	assert(old_player.is_queued_for_deletion() and old_player != current.player)
	playbacks.append(weakref(current.player.get_stream_playback()))
	network._decode_voice(frame("speaker",501,epoch,packet))
	network._decode_voice(frame("speaker",0,epoch+1,packet))
	assert(current.sequence == 1 and current.route == 1 and current.queue.size() == 1)
	var legacy := frame("speaker",2,epoch+1,packet)
	legacy[1] = 0
	network._decode_voice(legacy)
	network._decode_voice(frame("speaker",2,0,packet))
	assert(current.sequence == 1)
	network._decode_voice(frame("speaker",2,epoch+1,packet,1))
	network._decode_voice(frame("speaker",3,epoch+1,packet,1))
	assert(current.started and current.queue.size() == 3)
	network._decode_voice(frame("wrap",0xffffffff,epoch,packet))
	network._decode_voice(frame("wrap",0,epoch,packet))
	network._decode_voice(frame("wrap",0xfffffffe,epoch,packet))
	assert(voice.streams.wrap.sequence == 0 and voice.streams.wrap.queue.size() == 2)
	playbacks.append(weakref(voice.streams.wrap.player.get_stream_playback()))
	var deadline := Time.get_ticks_msec() + 1000
	while not current.queue.is_empty():
		assert(Time.get_ticks_msec() < deadline, "New epoch did not decode/drain PCM")
		await tree.process_frame
	assert(not voice.microphone_enabled)
	old_player = null
	current = {}
	encoder = null
	voice.free()
	network.free()
	assert(AudioServer.bus_count == baseline)
	deadline = Time.get_ticks_msec() + 2000
	while playbacks.any(func(reference: WeakRef) -> bool: return reference.get_ref() != null):
		assert(Time.get_ticks_msec() < deadline, "Retired voice epoch playback survived cleanup")
		await tree.process_frame
	print("PASS voice epoch restart, late/replayed packet rejection, route preservation, serial wrap, PCM drain and cleanup")
