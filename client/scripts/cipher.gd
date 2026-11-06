extends RefCounted
## AES-256-CBC + HMAC-SHA256 encrypt-then-MAC, independent directional keys.
var secret: PackedByteArray
var sent: Array[int] = [0, 0, 0]
var received: Array[int] = [0, 0, 0]
var crypto := Crypto.new()

func _init(master: PackedByteArray) -> void:
	secret = master

func key(direction: int, purpose: int) -> PackedByteArray:
	var hash := HashingContext.new()
	hash.start(HashingContext.HASH_SHA256)
	hash.update(secret)
	hash.update(PackedByteArray([direction, purpose]))
	return hash.finish()

func mac(data: PackedByteArray, direction: int) -> PackedByteArray:
	return crypto.hmac_digest(HashingContext.HASH_SHA256, key(direction, 1), data)

func seal(channel: int, direction: int, kind: int, payload: PackedByteArray) -> PackedByteArray:
	sent[channel] += 1
	var frame := PackedByteArray([1, channel, direction])
	frame.resize(11)
	frame.encode_u64(3, sent[channel])
	var iv := crypto.generate_random_bytes(16)
	frame.append_array(iv)
	var plain := PackedByteArray([kind])
	plain.append_array(payload)
	var padding := 16 - plain.size() % 16
	for i in padding:
		plain.append(padding)
	var aes := AESContext.new()
	aes.start(AESContext.MODE_CBC_ENCRYPT, key(direction, 0), iv)
	frame.append_array(aes.update(plain))
	aes.finish()
	frame.append_array(mac(frame, direction))
	return frame

func open(channel: int, direction: int, frame: PackedByteArray) -> PackedByteArray:
	if channel < 0 or channel >= 3 or frame.size() < 75 or frame.size() > 65536:
		return PackedByteArray()
	if frame.slice(0, 3) != PackedByteArray([1, channel, direction]):
		return PackedByteArray()
	var body := frame.slice(0, frame.size() - 32)
	if not crypto.constant_time_compare(mac(body, direction), frame.slice(frame.size() - 32)):
		return PackedByteArray()
	var seq := frame.decode_u64(3)
	if seq <= received[channel] or (body.size() - 27) % 16 != 0:
		return PackedByteArray()
	var aes := AESContext.new()
	aes.start(AESContext.MODE_CBC_DECRYPT, key(direction, 0), frame.slice(11, 27))
	var plain := aes.update(frame.slice(27, frame.size() - 32))
	aes.finish()
	var padding := int(plain[-1])
	if padding < 1 or padding > 16 or padding >= plain.size():
		return PackedByteArray()
	for i in range(plain.size() - padding, plain.size()):
		if plain[i] != padding:
			return PackedByteArray()
	plain.resize(plain.size() - padding)
	received[channel] = seq
	return plain
