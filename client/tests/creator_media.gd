extends RefCounted
const Runtime = preload("res://scripts/creator_runtime.gd")

class NetworkMock extends Node:
	var sent: Array[Dictionary] = []
	func command(kind: int, data: Dictionary) -> void:
		sent.append({"kind":kind,"data":data.duplicate(true)})

static func run() -> void:
	var tree: SceneTree = Engine.get_main_loop()
	var runtime := Runtime.new()
	tree.root.add_child(runtime)
	runtime.set_process(false)
	var root := Node3D.new()
	tree.root.add_child(root)
	var network := NetworkMock.new()
	root.add_child(network)
	root.set_meta("ssc_network", network)
	runtime.scope = root
	var errors: Array[String] = []
	runtime.script_error.connect(func(message: String): errors.append(message))
	var vm = ClassDB.instantiate("CreatorVm")
	assert(vm.load_script("""
assert(os == nil and world.spawn == nil)
world.on('action', function()
    world.media({target='screen',action='load',source='https://example.com/video.mp4'})
    world.media({target='screen',action='pause'})
    world.media({target='screen',action='seek',value=42})
    world.media({target='screen',action='play'})
end)
""", PackedStringArray(["media"])).is_empty())
	runtime.scripts.append({"vm":vm,"attachment":"world","failed":false})
	runtime.dispatch("action", {})
	assert(network.sent.size() == 4 and errors.is_empty())
	assert(network.sent[0].kind == 22 and network.sent[0].data.source == "https://example.com/video.mp4")
	assert(network.sent[2].data.action == "seek" and network.sent[2].data.value == 42)
	assert(not network.sent[0].data.has("actor") and not network.sent[0].data.has("owner"))
	var command := {"action":"media","target":"screen","request":{"target":"screen","action":"pause"}}
	runtime._apply(command, "other_surface")
	assert(network.sent.size() == 4)
	runtime._apply(command, "screen")
	assert(network.sent.size() == 5)
	for index in 20:
		runtime._apply(command, "world")
	assert(network.sent.size() == 8 and errors.size() == 1)
	runtime.media_window = Time.get_ticks_msec() - 1001
	runtime._apply(command, "world")
	assert(network.sent.size() == 9 and errors.size() == 1)
	root.remove_meta("ssc_network")
	runtime._apply(command, "world")
	assert(network.sent.size() == 9)
	runtime.close()
	assert(runtime.media_requests == 0)
	runtime.free()
	root.free()
	print("PASS creator media native bridge, authenticated request path, attachment isolation and aggregate client rate limit")
