extends RefCounted
const Network = preload("res://scripts/network.gd")

static func run() -> void:
	var network := Network.new()
	var statuses: Array[String] = []
	network.status.connect(func(text: String): statuses.append(text))
	network._decode(26,JSON.stringify({"send_hz":15,"receive_hz":30}).to_utf8_buffer())
	assert(network.send_hz == 60 and statuses.is_empty())
	network.joined = true
	network.elapsed = 10
	network._decode(26,JSON.stringify({"send_hz":22.5,"receive_hz":11.25}).to_utf8_buffer())
	assert(network.send_hz == 22.5 and network.receive_hz == 11.25)
	assert(network.elapsed <= 1.0 / 22.5 and statuses.size() == 1)
	for invalid in [{"send_hz":0,"receive_hz":30},{"send_hz":15,"receive_hz":121},{}]:
		network._decode(26,JSON.stringify(invalid).to_utf8_buffer())
	assert(network.send_hz == 22.5 and network.receive_hz == 11.25 and statuses.size() == 1)
	var samples: Array[Dictionary] = []
	network.message.connect(func(kind: int, data: Dictionary):
		if kind == 3:
			samples.append(data))
	var sample := JSON.stringify({"tick":7,"players":[],"entities":[]}).to_utf8_buffer()
	var middle := sample.size() >> 1
	var first := {"id":10,"index":0,"count":2,"data":sample.slice(0,middle).hex_encode()}
	var last := {"id":10,"index":1,"count":2,"data":sample.slice(middle).hex_encode()}
	network._decode(27,JSON.stringify(first).to_utf8_buffer())
	assert(samples.is_empty())
	network._decode(27,JSON.stringify(last).to_utf8_buffer())
	assert(samples.size() == 1 and int(samples[0].tick) == 7)
	first.id = 11
	network._decode(27,JSON.stringify(first).to_utf8_buffer())
	last.id = 12 # A new incomplete sample supersedes the old partial.
	network._decode(27,JSON.stringify(last).to_utf8_buffer())
	first.id = 11
	network._decode(27,JSON.stringify(first).to_utf8_buffer())
	assert(samples.size() == 1)
	first.id = 12
	network._decode(27,JSON.stringify(first).to_utf8_buffer())
	assert(samples.size() == 2 and network.pose_fragments.is_empty())
	network.close()
	assert(network.pose_fragments.is_empty() and network.pose_fragment_id == -1)
	network.free()
	print("PASS authenticated negotiated-rate updates, asymmetric fractional rates and invalid/prejoin rejection")
