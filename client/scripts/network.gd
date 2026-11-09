extends Node
signal message(kind: int, data: Dictionary)
signal status(text: String)
signal voice_packet(user: String, sequence: int, timestamp: int, packet: PackedByteArray, route: int, epoch: int)
signal negotiated(bitrate: int)
signal disconnected
const Cipher = preload("res://scripts/cipher.gd")
var host: ENetConnection
var peer: ENetPacketPeer
var cipher: RefCounted
var grant: Dictionary
var joined := false
var input_enabled := true
var pending_connection: Node

func set_input_focus(focused: bool) -> void:
	input_enabled = focused
	if is_instance_valid(pending_connection):
		pending_connection.set_input_focus(focused)
	elapsed = 0
	if joined:
		command(28,{"active":focused})
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
var pose_fragment_id := -1
var pose_fragments: Dictionary = {}
var pose_fragment_count := 0
var pose_fragment_bytes := 0
var pose_fragment_started := 0

func connect_grant(data: Dictionary) -> Error:
	if is_instance_valid(pending_connection):
		return ERR_BUSY
	if joined:
		if not data.has_all(["host", "port", "ticket", "secret"]) or str(data.secret).length() != 64 or str(data.ticket).length() != 64:
			return ERR_INVALID_DATA
		# Keep the current authenticated connection/resources live until the new
		# server has actually admitted us. Rejected transitions are not a leave.
		var candidate: Node = get_script().new()
		pending_connection = candidate
		candidate.input_enabled = input_enabled
		add_child(candidate)
		candidate.message.connect(_candidate_message)
		candidate.disconnected.connect(_candidate_disconnected)
		var error: Error = candidate._begin_connect(data)
		if error != OK:
			_discard_pending()
		else:
			status.emit("Connecting to the next instance; the current session remains live.")
		return error
	return _begin_connect(data)

func _begin_connect(data: Dictionary) -> Error:
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

func _discard_pending() -> void:
	if is_instance_valid(pending_connection):
		var candidate := pending_connection
		pending_connection = null
		candidate.close()
		candidate.queue_free()

func _candidate_disconnected() -> void:
	if not is_instance_valid(pending_connection):
		return
	_discard_pending()
	status.emit("New connection failed; the previous session is retained if still connected.")
	if not joined:
		disconnected.emit()

func _candidate_message(kind: int, data: Dictionary) -> void:
	if not is_instance_valid(pending_connection):
		return
	if kind == 255:
		_discard_pending()
		message.emit(kind,data)
		if not joined:
			disconnected.emit()
		return
	if kind != 2 or not data.has("user"):
		return
	var candidate := pending_connection
	pending_connection = null
	close(false)
	host = candidate.host
	peer = candidate.peer
	cipher = candidate.cipher
	user = candidate.user
	send_hz = candidate.send_hz
	receive_hz = candidate.receive_hz
	joined = true
	elapsed = 0
	since_packet = 0
	candidate.host = null
	candidate.peer = null
	candidate.cipher = null
	candidate.joined = false
	candidate.set_process(false)
	candidate.queue_free()
	negotiated.emit(int(data.get("voice_bitrate",24000)))
	status.emit("Joined · send %s Hz / receive %s Hz" % [send_hz,receive_hz])
	message.emit(kind,data)

func close(notify: bool = true) -> void:
	var was_connected := joined or host != null
	_discard_pending()
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
	_clear_pose_fragments()
	pose_fragment_id = -1
	if notify and was_connected:
		disconnected.emit()

func _clear_pose_fragments() -> void:
	pose_fragments.clear()
	pose_fragment_count = 0
	pose_fragment_bytes = 0
	pose_fragment_started = 0

func _process(delta: float) -> void:
	if not host:
		return
	since_packet += delta
	clock_elapsed += delta
	if pose_fragment_started > 0 and Time.get_ticks_msec() - pose_fragment_started > 2000:
		_clear_pose_fragments()
	if joined and clock_elapsed >= 2.0:
		clock_elapsed = 0
		command(23, {"sent":Time.get_ticks_msec() / 1000.0})
	for i in 128:
		var event := host.service()
		match event[0]:
			ENetConnection.EVENT_NONE:
				break
			ENetConnection.EVENT_ERROR, ENetConnection.EVENT_DISCONNECT:
				# service() has already retired the native peer on disconnect.
				peer = null
				# A successful takeover can retire the old peer before its new
				# snapshot reaches us. Keep the pending candidate alive.
				var candidate := pending_connection
				pending_connection = null
				close(candidate == null)
				pending_connection = candidate
				status.emit("Finishing the new authenticated connection…" if candidate != null else "Disconnected. Obtain a fresh grant; the instance survives only if someone remained.")
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
					_decode_voice(plain)
					continue
				_decode(int(plain[0]), plain.slice(1))
				if not host:
					return # A staged join transferred/retired this connection.
	if since_packet > 12.0:
		status.emit("Connection timed out. Local XR tracking remains active.")
		var candidate := pending_connection
		pending_connection = null
		close(candidate == null)
		pending_connection = candidate
		return
	host.flush()

func _exit_tree() -> void:
	close(false)

func _decode_voice(plain: PackedByteArray) -> void:
	# Bit 7 marks the epoch-bearing format; never mistake a legacy sequence or
	# timestamp for an epoch. Only authenticated server frames reach this method.
	if plain.size() < 3 or plain[0] != 20 or plain[1] not in [128,129]:
		return
	var size := int(plain[2])
	if size < 1 or size > 64 or plain.size() <= 23 + size:
		return
	var epoch := plain.decode_u64(3 + size)
	if epoch <= 0:
		return
	voice_packet.emit(plain.slice(3,3+size).get_string_from_utf8(), plain.decode_u32(11+size), plain.decode_u64(15+size), plain.slice(23+size), int(plain[1]) & 1, epoch)

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
	if kind == 27:
		var id := int(data.get("id",-1))
		var part := int(data.get("index",-1))
		var count := int(data.get("count",0))
		var encoded := str(data.get("data",""))
		if not joined or id < pose_fragment_id or part < 0 or part >= count or count < 1 or count > 256 or encoded.is_empty() or encoded.length() > 48000 or encoded.length() % 2 != 0 or not encoded.is_valid_hex_number(false):
			return
		if id > pose_fragment_id:
			_clear_pose_fragments()
			pose_fragment_id = id
			pose_fragment_count = count
			pose_fragment_started = Time.get_ticks_msec()
		if count != pose_fragment_count or pose_fragments.has(part):
			return
		var fragment := encoded.hex_decode()
		if pose_fragment_bytes + fragment.size() > 6 * 1024 * 1024:
			_clear_pose_fragments()
			return
		pose_fragments[part] = fragment
		pose_fragment_bytes += fragment.size()
		if pose_fragments.size() == count:
			var combined := PackedByteArray()
			for index in count:
				combined.append_array(pose_fragments[index])
			_clear_pose_fragments()
			_decode(3,combined)
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
		if not input_enabled:
			command(28,{"active":false})
		negotiated.emit(int(data.get("voice_bitrate", 24000)))
		status.emit("Joined · send %s Hz / receive %s Hz" % [send_hz, receive_hz])
	elif kind == 26:
		var input_rate := float(data.get("send_hz",0))
		var output_rate := float(data.get("receive_hz",0))
		if not joined or not is_finite(input_rate) or not is_finite(output_rate) or input_rate < 1 or input_rate > 120 or output_rate < 1 or output_rate > 120:
			return
		send_hz = input_rate
		receive_hz = output_rate
		elapsed = minf(elapsed,1.0 / send_hz)
		status.emit("Connection rates adjusted · send %s Hz / receive %s Hz" % [send_hz,receive_hz])
	message.emit(kind, data)

func command(kind: int, data: Dictionary) -> void:
	if joined and (input_enabled or kind not in [5,6,12,13,25]):
		peer.send(0, cipher.seal(0, 0, kind, JSON.stringify(data).to_utf8_buffer()), ENetPacketPeer.FLAG_RELIABLE)

func media_timeline(state: Dictionary) -> Dictionary:
	var adjusted := state.duplicate()
	if clock_ready and bool(state.get("playing", false)):
		var now := Time.get_ticks_msec() / 1000.0 + clock_offset
		var age := clampf(now - float(state.get("sampled_at", now)), 0, 10)
		adjusted.position = float(state.get("position", 0)) + age * float(state.get("rate", 1))
	return adjusted

func input_action(data: Dictionary) -> void:
	if joined and input_enabled:
		peer.send(1, cipher.seal(1, 0, 12, JSON.stringify(data).to_utf8_buffer()), 0)

func voice(sequence: int, timestamp: int, opus: PackedByteArray) -> void:
	if not joined or not input_enabled or opus.is_empty() or opus.size() > 1275:
		return
	var data := PackedByteArray()
	data.resize(12)
	data.encode_u32(0, sequence)
	data.encode_u64(4, timestamp)
	data.append_array(opus)
	peer.send(2, cipher.seal(2, 0, 20, data), ENetPacketPeer.FLAG_UNSEQUENCED)

func radio_voice(target: String, opus: PackedByteArray) -> void:
	if joined and input_enabled and not opus.is_empty() and opus.size() <= 104:
		var data := JSON.stringify({"target":target,"mode":"voice","data":opus.hex_encode()}).to_utf8_buffer()
		peer.send(2, cipher.seal(2, 0, 24, data), ENetPacketPeer.FLAG_UNSEQUENCED)

func pose(delta: float, transforms: Array[Transform3D], tracking: Dictionary = {}) -> void:
	if not joined or not input_enabled:
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
