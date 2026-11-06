extends Node
signal message(kind: int, data: Dictionary)
signal status(text: String)
signal voice_packet(user: String, sequence: int, timestamp: int, packet: PackedByteArray, route: int)
signal negotiated(bitrate: int)
const Cipher = preload("res://scripts/cipher.gd")
var host: ENetConnection
var peer: ENetPacketPeer
var cipher: RefCounted
var grant: Dictionary
var joined := false
var user := ""
var send_hz := 60.0
var receive_hz := 60.0
var elapsed := 0.0
var since_packet := 0.0
var fragments: Array[PackedByteArray] = []
var fragment_id := -1
var fragment_next := 0
var had_body_tracking := false
var clock_offset := 0.0
var clock_ready := false
var clock_elapsed := 2.0
var clock_best_rtt := INF

func connect_grant(data: Dictionary) -> Error:
	close()
	if not data.has_all(["host", "port", "ticket", "secret"]) or str(data.secret).length() != 64 or str(data.ticket).length() != 64:
		return ERR_INVALID_DATA
	grant = data.duplicate()
	cipher = Cipher.new(str(data.secret).hex_decode())
	host = ENetConnection.new()
	var error := host.create_host(1, 3)
	if error != OK:
		return error
	peer = host.connect_to_host(str(data.host), int(data.port), 3)
	status.emit("Connecting…")
	return OK

func close() -> void:
	clock_ready = false
	clock_best_rtt = INF
	clock_elapsed = 2.0
	joined = false
	if peer:
		peer.peer_disconnect_now()
	if host:
		host.destroy()
	peer = null
	host = null
	grant.clear()
	cipher = null
	since_packet = 0.0
	fragments.clear()
	fragment_id = -1

func _process(delta: float) -> void:
	if not host:
		return
	since_packet += delta
	clock_elapsed += delta
	if joined and clock_elapsed >= 2.0:
		clock_elapsed = 0
		command(23, {"sent":Time.get_ticks_msec() / 1000.0})
	for i in 128:
		var event := host.service()
		match event[0]:
			ENetConnection.EVENT_NONE:
				break
			ENetConnection.EVENT_ERROR, ENetConnection.EVENT_DISCONNECT:
				status.emit("Disconnected. Obtain a fresh grant; the instance survives only if someone remained.")
				close()
				return
			ENetConnection.EVENT_CONNECT:
				var first: PackedByteArray = str(grant.ticket).to_utf8_buffer()
				first.append_array(cipher.seal(0, 0, 1, PackedByteArray()))
				peer.send(0, first, ENetPacketPeer.FLAG_RELIABLE)
				grant.clear()
			ENetConnection.EVENT_RECEIVE:
				var packet: PackedByteArray = event[1].get_packet()
				var plain: PackedByteArray = cipher.open(int(event[3]), 1, packet)
				if plain.is_empty():
					continue
				since_packet = 0.0
				if plain[0] == 20:
					var size := int(plain[2]) if plain.size() > 2 else 0
					if size > 0 and plain.size() >= 15 + size:
						voice_packet.emit(plain.slice(3, 3 + size).get_string_from_utf8(), plain.decode_u32(3 + size), plain.decode_u64(7 + size), plain.slice(15 + size), int(plain[1]))
					continue
				_decode(int(plain[0]), plain.slice(1))
	if since_packet > 12.0:
		status.emit("Connection timed out. Local XR tracking remains active.")
		close()
		return
	host.flush()

func _decode(kind: int, bytes: PackedByteArray) -> void:
	var data = JSON.parse_string(bytes.get_string_from_utf8())
	if not data is Dictionary:
		return
	if kind == 23:
		var now := Time.get_ticks_msec() / 1000.0
		var rtt := now - float(data.get("sent", now))
		if rtt >= 0 and rtt < clock_best_rtt:
			clock_best_rtt = rtt
			clock_offset = float(data.get("server", 0)) - now + rtt / 2.0
			clock_ready = true
		return
	if kind == 10:
		if not data.has_all(["id", "index", "count", "kind", "data"]) or int(data.count) < 1 or int(data.count) > 256 or str(data.data).length() > 48000:
			return
		if int(data.id) != fragment_id:
			if int(data.index) != 0:
				return
			fragment_id = int(data.id)
			fragment_next = 0
			fragments.clear()
		if int(data.index) != fragment_next:
			return
		fragments.append(str(data.data).hex_decode())
		fragment_next += 1
		if fragment_next == int(data.count):
			var combined := PackedByteArray()
			for part in fragments:
				combined.append_array(part)
			fragments.clear()
			if int(data.kind) != 10:
				_decode(int(data.kind), combined)
		return
	if kind == 2 and data.has("user"):
		user = str(data.user)
		send_hz = float(data.send_hz)
		receive_hz = float(data.receive_hz)
		joined = true
		negotiated.emit(int(data.get("voice_bitrate", 24000)))
		status.emit("Joined · send %s Hz / receive %s Hz" % [send_hz, receive_hz])
	message.emit(kind, data)

func command(kind: int, data: Dictionary) -> void:
	if joined:
		peer.send(0, cipher.seal(0, 0, kind, JSON.stringify(data).to_utf8_buffer()), ENetPacketPeer.FLAG_RELIABLE)

func media_timeline(state: Dictionary) -> Dictionary:
	var adjusted := state.duplicate()
	if clock_ready and bool(state.get("playing", false)):
		var now := Time.get_ticks_msec() / 1000.0 + clock_offset
		var age := clampf(now - float(state.get("sampled_at", now)), 0, 10)
		adjusted.position = float(state.get("position", 0)) + age * float(state.get("rate", 1))
	return adjusted

func input_action(data: Dictionary) -> void:
	if joined:
		peer.send(1, cipher.seal(1, 0, 12, JSON.stringify(data).to_utf8_buffer()), 0)

func voice(sequence: int, timestamp: int, opus: PackedByteArray) -> void:
	if not joined or opus.is_empty() or opus.size() > 1275:
		return
	var data := PackedByteArray()
	data.resize(12)
	data.encode_u32(0, sequence)
	data.encode_u64(4, timestamp)
	data.append_array(opus)
	peer.send(2, cipher.seal(2, 0, 20, data), ENetPacketPeer.FLAG_UNSEQUENCED)

func radio_voice(target: String, opus: PackedByteArray) -> void:
	if joined and not opus.is_empty() and opus.size() <= 104:
		var data := JSON.stringify({"target":target,"mode":"voice","data":opus.hex_encode()}).to_utf8_buffer()
		peer.send(2, cipher.seal(2, 0, 24, data), ENetPacketPeer.FLAG_UNSEQUENCED)

func pose(delta: float, transforms: Array[Transform3D], tracking: Dictionary = {}) -> void:
	if not joined:
		return
	elapsed += delta
	if elapsed < 1.0 / send_hz:
		return
	elapsed = fmod(elapsed, 1.0 / send_hz)
	var bytes := PackedByteArray()
	bytes.resize(84)
	var offset := 0
	for t in transforms:
		var q := t.basis.get_rotation_quaternion()
		for v in [t.origin.x, t.origin.y, t.origin.z, q.x, q.y, q.z, q.w]:
			bytes.encode_float(offset, v)
			offset += 4
	peer.send(1, cipher.seal(1, 0, 3, bytes), 0)
	if not tracking.is_empty() or had_body_tracking:
		peer.send(1, cipher.seal(1, 0, 14, JSON.stringify(tracking).to_utf8_buffer()), 0)
	had_body_tracking = not tracking.is_empty()
