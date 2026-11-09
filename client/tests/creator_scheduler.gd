extends RefCounted
const Runtime = preload("res://scripts/creator_runtime.gd")

class Vm extends RefCounted:
	var events: Array[String] = []
	var closed := false
	func dispatch(event: String, _encoded: String) -> String:
		events.append(event)
		return '{"commands":[]}'
	func close() -> void:
		closed = true

static func run() -> void:
	var tree: SceneTree = Engine.get_main_loop()
	var scope := Node3D.new()
	tree.root.add_child(scope)
	var first := Runtime.new()
	var second := Runtime.new()
	tree.root.add_child(first)
	tree.root.add_child(second)
	first.scope = scope
	second.scope = scope
	var a := Vm.new()
	var b := Vm.new()
	first.scripts.append({"vm":a,"attachment":"world","failed":false})
	second.scripts.append({"vm":b,"attachment":"world","failed":false})
	first.set_process(true)
	second.set_process(true)
	first.dispatch("join",{"id":"a"})
	second.dispatch("join",{"id":"b"})
	assert(a.events.is_empty() and b.events.is_empty())
	Runtime.scheduler_cursor = 0
	Runtime.advance_all(0.1,1_000_000,2,1_000_000)
	assert(a.events.is_empty() and b.events.is_empty()) # One refresh step each.
	Runtime.advance_all(0.1,1_000_001,4,1_000_000)
	assert(a.events == ["join"] and b.events == ["join"])
	Runtime.advance_all(0.1,1_000_001,100,1_000_000)
	assert(a.events == ["join"] and b.events == ["join"]) # Same-frame dedup.
	Runtime.advance_all(0.1,1_000_002,100,0)
	assert(a.events == ["join"] and b.events == ["join"]) # Exhausted frame yields.
	Runtime.advance_all(0.1,1_000_003,100,1_000_000)
	assert(a.events == ["join","tick"] and b.events == ["join","tick"])
	first.dispatch("leave",{"id":"a"})
	first.close()
	assert(first.jobs.is_empty() and first.job_bytes == 0 and a.closed)
	first.free()
	second.free()
	scope.free()
	assert(b.closed)
	Runtime.scheduler_frame = -1
	print("PASS fair aggregate creator scheduler, preserved callback order, bounded frame work, no tick backlog and teardown")
