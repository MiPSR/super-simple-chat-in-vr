extends SceneTree
const Radio = preload("res://scripts/radio.gd")
const Creator = preload("res://scripts/creator_runtime.gd")
class Transport extends Node:
	var joined := true
	var user := "listener"
	var commands: Array = []
	func command(kind: int, data: Dictionary) -> void:
		commands.append({"kind":kind,"data":data})
var started := 0

func _initialize() -> void:
	started = Time.get_ticks_msec()
	call_deferred("_run")

func _run() -> void:
	var baseline := AudioServer.bus_count
	var network := Transport.new()
	root.add_child(network)
	network.joined = true
	network.user = "listener"
	var probe := Radio.new()
	root.add_child(probe)
	probe.attach(network)
	probe.configure([{"id":"strict","position":[0,0,0],"channel":1,"preset":"weak"},{"id":"fallback","position":[1,0,0],"channel":1,"preset":"standard"}], {"noise":0.2,"filter_hz":900,"presets":["narrow","weak"]})
	assert(probe.devices.strict.preset == "weak" and probe.devices.fallback.preset == "narrow")
	var strict_index := AudioServer.get_bus_index(probe.devices.strict.bus)
	assert(is_equal_approx(AudioServer.get_bus_effect(strict_index, 0).cutoff_hz,450))
	assert(AudioServer.get_bus_send(strict_index) == probe.bus)
	assert(is_equal_approx(float(probe.devices.strict.noise),0.5))
	probe.free()
	assert(AudioServer.bus_count == baseline)
	var radio := Radio.new()
	root.add_child(radio)
	radio.attach(network)
	radio.configure([{"id":"desk","position":[0,1,-1],"channel":1}], {"noise":0.2,"filter_hz":900,"interference":0.4})
	assert(is_equal_approx(AudioServer.get_bus_effect(AudioServer.get_bus_index(radio.bus),0).cutoff_hz,900))
	assert(radio.apply_preset("weak"))
	assert(is_equal_approx(AudioServer.get_bus_effect(AudioServer.get_bus_index(radio.bus),0).cutoff_hz,450))
	assert(radio.noise >= 0.2 and not radio.apply_preset("unbounded"))
	assert(radio.apply_preset("standard"))
	radio.allowed_presets = ["narrow"]
	assert(not radio.apply_preset("standard") and radio.apply_preset("narrow"))
	radio.allowed_presets = ["standard", "narrow", "weak"]
	assert(radio.apply_preset("standard"))
	var scope := Node3D.new()
	root.add_child(scope)
	scope.set_meta("ssc_radio", radio)
	var creator := Creator.new()
	scope.add_child(creator)
	creator.scope = scope
	creator._apply({"action":"radio","target":"desk","mode":"ft8","data":"CQ CREATOR"}, "world")
	assert(network.commands == [{"kind":25,"data":{"target":"desk","mode":"ft8","data":"CQ CREATOR"}}])
	assert(not radio.transmit("desk", "ft8", "CQ AGAIN"))
	assert(network.commands.size() == 1)
	assert(not radio.transmit("missing", "ft8", "CQ"))
	assert(not radio.transmit("desk", "voice", "00"))
	assert(not radio.transmit("desk", "ft8", "猫"))
	assert(not radio.transmit("desk", "sstv", "00"))
	assert(not radio.transmit("desk", "sstv", "zz".repeat(3072)))
	network.joined = false
	assert(not radio.transmit("desk", "ft8", "CQ"))
	network.joined = true
	scope.free()
	radio.receive({"radio":"desk","mode":"ft8","sender":"remote","data":"CQ VR","sequence":1})
	assert(radio.devices.desk.label.text.contains("CQ VR"))
	var bytes := PackedByteArray()
	bytes.resize(64*48)
	bytes.fill(160)
	radio.receive({"radio":"desk","mode":"sstv","sender":"remote","data":bytes.hex_encode(),"sequence":2,"interference":true})
	assert(radio.devices.desk.label.text.contains("SSTV"))
	assert(is_equal_approx(float(radio.devices.desk.noise),0.6))
	var codec = ClassDB.instantiate("OpusCodec")
	assert(codec.configure(8000))
	var pcm := PackedVector2Array()
	pcm.resize(960)
	pcm.fill(Vector2(0.1,0.1))
	for sequence in [3,4,5]:
		radio.receive({"radio":"desk","mode":"voice","sender":"remote","data":codec.encode(pcm).hex_encode(),"sequence":sequence})
	assert(radio.receiver.streams.size() == 1)
	var player: AudioStreamPlayer3D = radio.receiver.streams["desk:remote"].player
	assert(player.global_position.is_equal_approx(Vector3(0,1,-1)) and player.bus == radio.devices.desk.bus)
	assert(AudioServer.get_bus_send(AudioServer.get_bus_index(player.bus)) == radio.bus)
	radio.receiver.prefer_radio("desk:remote")
	assert(player.volume_db < -70)
	await process_frame
	await process_frame
	assert(radio.devices.desk.tone.playing)
	var playbacks := [weakref(player.get_stream_playback()), weakref(radio.devices.desk.tone.get_stream_playback())]
	player = null
	codec = null
	radio.free()
	network.free()
	assert(AudioServer.bus_count == baseline)
	# AudioServer fades stopped streams on its mixer thread, then releases them
	# on the main thread. Observe that release instead of assuming 100 ms is
	# enough on every loaded host; a genuine leak still fails the deadline.
	var cleanup_deadline := Time.get_ticks_msec() + 2000
	while playbacks.any(func(reference: WeakRef) -> bool: return reference.get_ref() != null):
		assert(Time.get_ticks_msec() < cleanup_deadline, "Radio playbacks survived cleanup")
		await process_frame
	print("PASS radio 3D speaker, protected SSTV texture/FT8 display, Opus queue, filters and cleanup")
	quit()

func _process(_delta: float) -> bool:
	if Time.get_ticks_msec() - started > 5000:
		quit(2)
	return false
