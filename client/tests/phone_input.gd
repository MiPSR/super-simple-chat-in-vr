extends RefCounted
class Transport extends Node:
	var joined := false
	var sent: Array = []
	func command(kind: int, data: Dictionary) -> void:
		sent.append({"kind":kind,"data":data})

static func run() -> void:
	var tree: SceneTree = Engine.get_main_loop()
	var app: Node = load("res://main.tscn").instantiate()
	tree.root.add_child(app)
	var line: LineEdit = app.chat_input
	app.keyboard_target = line
	line.text = "ac"
	line.caret_column = 1
	app._key_pressed("b", "b")
	assert(line.text == "abc" and line.caret_column == 2)
	app._key_pressed("backspace", "")
	assert(line.text == "ac" and line.caret_column == 1)
	line.max_length = 2
	line.select(0, 1)
	app._key_pressed("z", "z")
	assert(line.text == "zc", "Selection replacement must work even at the character limit")
	line.select(0, 1)
	app._key_pressed("backspace", "")
	assert(line.text == "c")
	line.max_length = 512
	line.clear()
	app._key_pressed("shift", "")
	app._key_pressed("a", "a")
	app._key_pressed("shift", "")
	for key in [":","/","?","&","=","_","%","#","@","+"]:
		app._key_pressed(key, key)
	assert(line.text == "A:/?&=_%#@+")
	line.editable = false
	app._key_pressed("b", "b")
	assert(line.text == "A:/?&=_%#@+")
	app._phone_focus_changed(line)
	assert(app.keyboard_target == null)
	line.editable = true
	# Invalid messages remain editable; a valid message uses the reliable path.
	var real_transport: Node = app.network
	var transport := Transport.new()
	app.add_child(transport)
	app.network = transport
	line.text = "hello"
	app._send_chat(line.text)
	assert(line.text == "hello" and transport.sent.is_empty())
	transport.joined = true
	line.text = "界".repeat(171)
	app._send_chat(line.text)
	assert(not line.text.is_empty() and transport.sent.is_empty())
	app._send_chat("  hello  ")
	assert(line.text.is_empty() and transport.sent[-1] == {"kind":4,"data":{"text":"hello"}})
	# Teleport/respawn changes the virtual floor, never the tracking transform.
	app.camera.position = Vector3(0.7, 1.2, -0.4)
	var tracked: Transform3D = app.camera.transform
	app.locomotion.velocity = Vector3(0,-20,0)
	app._teleport_to_spawn(Vector3(5, 3, 7))
	assert(app.camera.transform == tracked and app.locomotion.velocity == Vector3.ZERO)
	assert(app.camera.global_position.is_equal_approx(Vector3(5,4.2,7)))
	var world := Node3D.new()
	app.add_child(world)
	app.world_root = world
	app.loader.manifest = {"spawn":[1,2,3]}
	app.selected_entity = "prop"
	app._respawn()
	assert(app.camera.global_position.is_equal_approx(Vector3(1,3.2,3)))
	assert(app.selected_entity.is_empty() and transport.sent[-1] == {"kind":12,"data":{"action":"release","target":"prop"}})
	app.network = real_transport
	app.free()
	print("PASS phone keyboard caret/selection/shift/URL symbols, UTF-8 chat limits and explicit tracking-preserving respawn")
