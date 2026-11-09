extends RefCounted
const Voice = preload("res://scripts/voice.gd")

static func run() -> void:
	var tree: SceneTree = Engine.get_main_loop()
	var voice := Voice.new()
	tree.root.add_child(voice)
	voice.set_process(false)
	var player := AudioStreamPlayer3D.new()
	voice.add_child(player)
	voice.streams["idle-user"] = {"player":player,"route":0}
	voice.positions([],false)
	assert(voice.streams.has("idle-user"))
	voice.positions([{"id":"idle-user","pose":[1,2,3]}],false)
	assert(player.global_position.is_equal_approx(Vector3(1,2,3)))
	voice.positions([],true)
	assert(voice.streams.is_empty())
	voice.free()
	print("PASS partial pose updates preserve idle voice streams; complete rosters still retire departed proximity speakers")
