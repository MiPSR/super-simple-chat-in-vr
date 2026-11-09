extends RefCounted
const Humanoid = preload("res://scripts/humanoid.gd")
const Runtime = preload("res://scripts/creator_runtime.gd")

static func model(name: String, offset: float) -> Node3D:
	var root := Node3D.new()
	root.name = name
	var skeleton := Skeleton3D.new()
	skeleton.name = "Skeleton"
	skeleton.add_bone("Head")
	skeleton.add_bone("HairRoot")
	root.add_child(skeleton)
	var hat := Node3D.new()
	hat.name = "Hat"
	hat.position.x = offset
	root.add_child(hat)
	return root

static func write(zip: ZIPPacker, path: String, source: String) -> void:
	assert(zip.start_file(path) == OK)
	assert(zip.write_file(source.to_utf8_buffer()) == OK)
	assert(zip.close_file() == OK)

static func run() -> void:
	var tree: SceneTree = Engine.get_main_loop()
	var path := "user://avatar-queries-%s-%s.zip" % [OS.get_process_id(),Time.get_ticks_usec()]
	var zip := ZIPPacker.new()
	assert(zip.open(path) == OK)
	write(zip,"queries.luau","""
assert(avatar.object('node:0').name == 'Model')
assert(avatar.object('node:2').name == 'Hat' and avatar.object('node:2').get_node == nil)
assert(avatar.object('bone:0').name == 'Head' and avatar.object('humanoid:Head').name == 'Head')
assert(avatar.object('other') == nil and avatar.state.get('world_secret') == nil)
local ids, cursor = avatar.find_tag('avatar.bone'); assert(#ids == 2 and cursor == nil)
avatar.set_visible('node:2',false)
avatar.on('action',function(e)
    if e.deleted then
        assert(avatar.object('node:2') == nil)
        local nodes = avatar.find_tag('avatar.node'); assert(#nodes == 2)
    else
        local p = avatar.object('node:2'); assert(not p.visible and p.position[1] == e.expected)
        p.position[1] = 999; assert(avatar.object('node:2').position[1] == e.expected)
    end
    if e.reset then avatar.reset_bone('bone:1') else avatar.bone_rotation('bone:1',0,0.5,0,0.5) end
end)
""")
	write(zip,"deny.luau","assert(avatar.object('node:0') == nil); local ids = avatar.find_tag('avatar.node'); assert(#ids == 0)")
	write(zip,"other.luau","""
assert(avatar.object('node:0').name == 'Other' and avatar.object('node:2').position[1] == 9)
avatar.on('action',function() assert(avatar.object('node:0').name == 'Other') end)
""")
	assert(zip.close() == OK)
	var first := Humanoid.new()
	tree.root.add_child(first)
	first.setup(model("Model",1),true)
	var second := Humanoid.new()
	tree.root.add_child(second)
	second.setup(model("Other",9),true)
	var runtime := Runtime.new()
	tree.root.add_child(runtime)
	runtime.set_process(false)
	var errors: Array[String] = []
	runtime.script_error.connect(func(message: String): errors.append(message))
	runtime.load_package(path,{"scripts":[{"path":"queries.luau","side":"client","attachment":"avatar","capabilities":["visual","queries"]},{"path":"deny.luau","side":"client","attachment":"avatar","capabilities":["visual"]}]},first)
	assert(runtime.scripts.size() == 2 and errors.is_empty())
	var hat: Node3D = first.model_root.get_node("Hat")
	assert(not hat.visible)
	hat.position.x = 2
	hat.scale = Vector3.ZERO # Singular hidden transforms must not log engine errors.
	runtime.dispatch("action",{"expected":2})
	first.bone_modifier._process_modification()
	assert(first.skeleton.get_bone_pose_rotation(1).is_equal_approx(Quaternion(0,sqrt(0.5),0,sqrt(0.5))))
	assert(second.skeleton.get_bone_pose_rotation(1).is_equal_approx(Quaternion.IDENTITY))
	var other := Runtime.new()
	tree.root.add_child(other)
	other.set_process(false)
	other.script_error.connect(func(message: String): errors.append(message))
	other.load_package(path,{"scripts":[{"path":"other.luau","side":"client","attachment":"avatar","capabilities":["queries"]}]},second)
	assert(other.scripts.size() == 1 and errors.is_empty())
	hat.free()
	runtime.dispatch("action",{"deleted":true,"reset":true})
	assert(first.bone_modifier.rotations.is_empty() and errors.is_empty())
	assert(first.creator_node("node:2") == null)
	assert(first.creator_node("node:00") == null and first.creator_node("node:../0") == null)
	var scene := runtime.scene
	runtime.close()
	var probe = ClassDB.instantiate("CreatorVm")
	probe.bind_scene(scene)
	assert(probe.load_script("assert(avatar.object('node:0') == nil)",PackedStringArray()).is_empty())
	probe.close()
	other.dispatch("action",{})
	assert(errors.is_empty())
	first.query_nodes.resize(4097)
	assert(first.creator_query_snapshot().has("error"))
	runtime.free()
	other.free()
	first.free()
	second.free()
	assert(DirAccess.remove_absolute(ProjectSettings.globalize_path(path)) == OK)
	print("PASS opt-in own-avatar node/bone queries, live detached poses, numeric IDs, capability isolation, deletion and teardown")
