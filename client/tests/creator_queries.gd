extends RefCounted
const Runtime = preload("res://scripts/creator_runtime.gd")

static func run() -> void:
	var tree: SceneTree = Engine.get_main_loop()
	var runtime := Runtime.new()
	tree.root.add_child(runtime)
	runtime.set_process(false)
	var root := Node3D.new()
	var prop := Node3D.new()
	prop.name = "prop"
	prop.visible = false
	root.add_child(prop)
	tree.root.add_child(root)
	var errors: Array[String] = []
	runtime.script_error.connect(func(message: String): errors.append(message))
	runtime.world_queries = true
	runtime.update_objects({"prop":{"tags":["game.prop"],"position":[1,0,0],"controller":null}}, 10)
	runtime.update_state({"ready":false,"count":0,"waypoint":[1,2,3]}, 10)
	runtime.queue_world_events([{"event":"join","id":"new","clock":10}])
	runtime._dispatch_world_events()
	assert(runtime.pending_events.size() == 1, "Loading discarded a committed membership event")
	var path := "user://creator-query-test.zip"
	var zip := ZIPPacker.new()
	assert(zip.open(path) == OK)
	assert(zip.start_file("query.luau") == OK)
	assert(zip.write_file("""
assert(world.object('prop').position[1] == 1)
assert(world.object('missing') == nil and world.object('prop').controller == nil)
assert(world.spawn == nil and ClassDB == nil)
assert(world.state.set == nil and world.state.get('missing') == nil)
assert(world.state.get('ready') == false and world.state.get('count') == 0)
local waypoint = world.state.get('waypoint'); waypoint[1] = 999
assert(world.state.get('waypoint')[1] == 1)
world.set_visible('prop',true)
world.on('join',function(e) assert(e.id == 'new'); world.set_visible('prop',false) end)
world.on('leave',function(e) assert(e.id == 'new'); world.set_visible('prop',true) end)
for _, name in ipairs({'grab','lease','release','damage','destroy','collision'}) do
    world.on(name, function(e)
        assert(e.event == name and world.object('prop') == nil)
        if name == 'lease' then assert(e.kind == 'object' and e.hand == nil) end
        world.set_visible('prop', e.visible)
    end)
end
world.on('action', function(e)
    if e.state ~= nil then assert(world.state.get('count') == e.state) end
    local ids, cursor = world.find_tag('game.prop')
    assert(cursor == nil)
    if e.deleted then
        assert(#ids == 0 and world.object('prop') == nil)
    else
        assert(#ids == 1 and ids[1] == 'prop')
        local p = world.object('prop')
        assert(p.position[1] == e.expected)
        p.position[1] = 999
        p.tags[1] = 'modified'
        assert(world.object('prop').position[1] == e.expected)
        assert(world.object('prop').tags[1] == 'game.prop')
    end
    world.set_visible('prop', e.visible)
end)
""".to_utf8_buffer()) == OK)
	assert(zip.close_file() == OK)
	assert(zip.close() == OK)
	var declaration := {"side":"client","path":"query.luau","attachment":"world","capabilities":["visual"]}
	runtime.load_package(path, {"scripts":[declaration, declaration]}, root, true)
	assert(runtime.scripts.size() == 2 and errors.is_empty())
	assert(prop.visible and runtime.pending_events.size() == 1)
	runtime._dispatch_world_events()
	assert(not prop.visible and errors.is_empty() and runtime.pending_events.is_empty())
	runtime.dispatch("action", {"expected":1,"visible":false,"state":0})
	assert(not prop.visible and errors.is_empty())
	runtime.update_transforms([{"id":"prop","position":[3,0,0],"rotation":[0,0,0,1]}], 12)
	runtime.update_objects({"prop":{"tags":["game.prop"],"position":[2,0,0]}}, 11)
	runtime.update_state({"count":1}, 11)
	runtime.update_state({"count":0}, 9)
	runtime.dispatch("action", {"expected":3,"visible":true,"state":1})
	assert(prop.visible and errors.is_empty())
	var isolated = ClassDB.instantiate("CreatorVm")
	assert(isolated.load_script("assert(world.object('prop') == nil); local ids = world.find_tag('game.prop'); assert(#ids == 0)", PackedStringArray()).is_empty())
	isolated.close()
	var retired: WeakRef = weakref(runtime.scripts[0].vm)
	runtime.update_objects({}, 13)
	runtime.update_objects({"prop":{"tags":["game.prop"],"position":[1,0,0]}}, 10)
	runtime.update_transforms([{"id":"prop","position":[4,0,0],"rotation":[0,0,0,1]}], 14)
	runtime.dispatch("action", {"deleted":true,"visible":false})
	assert(not prop.visible and errors.is_empty())
	runtime.queue_world_events([{"event":"lease","kind":"object","actor":"a","target":"prop","visible":true}])
	assert(not prop.visible and runtime.pending_events.size() == 1)
	runtime._dispatch_world_events()
	assert(prop.visible and runtime.pending_events.is_empty() and errors.is_empty())
	runtime.queue_world_events([{"event":"collision","a":"prop","b":"floor","visible":false}])
	runtime._dispatch_world_events()
	assert(not prop.visible and errors.is_empty())
	runtime.queue_world_events([{"event":"leave","id":"new"}])
	runtime._dispatch_world_events()
	assert(prop.visible and errors.is_empty())
	var burst: Array = []
	for index in 129:
		burst.append({"event":"damage","target":"prop","visible":false})
	runtime.queue_world_events(burst)
	assert(runtime.pending_events.size() == 128)
	runtime._dispatch_world_events()
	assert(not prop.visible and runtime.pending_events.size() >= 120 and errors.size() == 1)
	runtime.close()
	assert(runtime.pending_events.is_empty())
	assert(retired.get_ref() == null)
	var vm = ClassDB.instantiate("CreatorVm")
	vm.bind_scene(runtime.scene)
	assert(vm.load_script("assert(world.object('prop') == nil and world.state.get('count') == nil)", PackedStringArray()).is_empty())
	vm.close()
	runtime.world_queries = true
	var many: Dictionary = {}
	for index in 300:
		many["obj%04d" % index] = {"tags":["game.prop"]}
	runtime.update_objects(many, 19)
	vm.bind_scene(runtime.scene)
	assert(vm.load_script("""
local ids, cursor = world.find_tag('game.prop')
assert(#ids == 256 and ids[1] == 'obj0000' and cursor == 'obj0255')
local tail, done = world.find_tag('game.prop', cursor)
assert(#tail == 44 and tail[1] == 'obj0256' and done == nil)
""", PackedStringArray()).is_empty())
	vm.close()
	runtime.update_objects({"bad":{"tags":["invalid tag"]}}, 20)
	assert(errors.size() == 2)
	vm.bind_scene(runtime.scene)
	assert(vm.load_script("local ids = world.find_tag('game.prop'); assert(#ids == 0)", PackedStringArray()).is_empty())
	vm.close()
	runtime.free()
	root.free()
	assert(DirAccess.remove_absolute(ProjectSettings.globalize_path(path)) == OK)
	print("PASS client detached object/state queries, live poses/deletion, isolation, nil, bounded deferred events and teardown")
