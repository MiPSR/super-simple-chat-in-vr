extends RefCounted
const Network = preload("res://scripts/network.gd")

class Candidate extends Node:
	var host: ENetConnection
	var peer: ENetPacketPeer
	var cipher: RefCounted = RefCounted.new()
	var user := "next-user"
	var send_hz := 15.0
	var receive_hz := 30.0
	var joined := true
	var closed := false
	func close() -> void:
		closed = true

static func run() -> void:
	var network := Network.new()
	network.joined = true
	network.user = "old-user"
	network.cipher = RefCounted.new()
	var previous := network.cipher
	var counts := {"disconnects":0,"negotiations":0}
	network.disconnected.connect(func(): counts.disconnects += 1)
	network.negotiated.connect(func(_bitrate: int): counts.negotiations += 1)
	var rejected := Candidate.new()
	network.add_child(rejected)
	network.pending_connection = rejected
	network._candidate_message(255,{"error":"Not admitted"})
	assert(network.joined and network.user == "old-user" and network.cipher == previous)
	assert(counts.disconnects == 0 and rejected.closed and network.pending_connection == null)
	var admitted := Candidate.new()
	network.add_child(admitted)
	var next_cipher := admitted.cipher
	network.pending_connection = admitted
	network._candidate_message(2,{"user":"next-user","voice_bitrate":24000,"snapshot":{}})
	assert(network.joined and network.user == "next-user" and network.cipher == next_cipher)
	assert(network.send_hz == 15 and network.receive_hz == 30)
	assert(counts.disconnects == 0 and counts.negotiations == 1)
	assert(network.pending_connection == null and admitted.cipher == null and not admitted.joined)
	network.close()
	assert(counts.disconnects == 1 and not network.joined)
	network.free()
	print("PASS staged connection rejection preserves old session, successful admission transfers transport and close notifies once")
