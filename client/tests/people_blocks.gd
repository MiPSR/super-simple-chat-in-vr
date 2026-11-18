extends RefCounted
const Main = preload("res://scripts/main.gd")
const Materials = preload("res://scripts/materials.gd")
const Visuals = preload("res://scripts/visual_overrides.gd")
const People = preload("res://scripts/people_tab.gd")

static func run() -> void:
	var tree: SceneTree = Engine.get_main_loop()
	var app: Node = load("res://main.tscn").instantiate()
	tree.root.add_child(app)
	var failures: Array = []
	app.loader.failed.connect(func(message: String): failures.append(message))
	# (a) Revision resnapshots omit "manifest"; the loaded world must survive.
	app.current_instance = "i1"
	app.current_version = "v1"
	var world := Node3D.new()
	app.add_child(world)
	app.world_root = world
	var manifestless := {"instance":"i1", "world":"w", "version":"v1", "tick":7, "players":{}, "entities":{}, "variables":{}}
	app._message(2, {"user":"u1", "snapshot":manifestless})
	assert(app.world_root == world and app.current_version == "v1" and failures.is_empty())
	assert(app.entity_data.is_empty() and app.latest_tick == 7)
	var stale := manifestless.duplicate()
	stale.version = "v2"
	app._message(2, {"user":"u1", "snapshot":stale})
	assert(app.world_root == world and app.current_version == "v1" and failures.is_empty(), "A manifest-less snapshot must not reload the world")
	var fresh := {"instance":"i1", "world":"w", "version":"v3", "tick":8, "players":{}, "entities":{}, "manifest":{"spawn":[5, 1, 2], "scripts":[]}}
	app._message(2, {"user":"u1", "snapshot":fresh})
	assert(app.current_version == "v3" and failures.size() == 1)
	assert(app.origin.position.is_equal_approx(Vector3(5, 1, 2)))
	# A snapshot carrying no fields at all only clears the instance; guarded
	# access keeps the client state consistent without errors.
	app._message(2, {"user":"u2"})
	assert(app.current_instance == "" and app.world_root == null and app.current_version == "")
	# (b) Visual overrides: tint multiplies and restores the stored base,
	# light rewrites and restores, animation plays and stops by name.
	var node := Node3D.new()
	tree.root.add_child(node)
	var mesh := MeshInstance3D.new()
	mesh.name = "Mesh"
	mesh.mesh = BoxMesh.new()
	var original := StandardMaterial3D.new()
	original.albedo_color = Color(0.5, 0.5, 0.5)
	mesh.material_override = original
	node.add_child(mesh)
	var light := OmniLight3D.new()
	node.add_child(light)
	var player := AnimationPlayer.new()
	node.add_child(player)
	var spin := Animation.new()
	spin.length = 1.0
	var track := spin.add_track(Animation.TYPE_VALUE)
	spin.track_set_path(track, NodePath("Mesh:position"))
	spin.track_insert_key(track, 0.0, Vector3.ZERO)
	spin.track_insert_key(track, 1.0, Vector3.ONE)
	var library := AnimationLibrary.new()
	library.add_animation("spin", spin)
	player.add_animation_library("", library)
	Materials.apply(mesh, 1)
	assert(mesh.get_active_material(0).albedo_color.is_equal_approx(Color(0.5, 0.5, 0.5)))
	Visuals.apply(node, {"tint":[1.0, 0.5, 0.5], "light":[1.0, 0.5, 0.25, 3.0], "animation":"spin", "animation_playing":true})
	assert(mesh.get_active_material(0).albedo_color.is_equal_approx(Color(0.5, 0.25, 0.25)))
	assert(light.light_color.is_equal_approx(Color(1.0, 0.5, 0.25)) and is_equal_approx(light.light_energy, 3.0))
	assert(player.is_playing() and player.current_animation == "spin")
	Visuals.apply(node, {"animation":"spin"})
	assert(not player.is_playing())
	Visuals.apply(node, {})
	assert(mesh.get_active_material(0).albedo_color.is_equal_approx(Color(0.5, 0.5, 0.5)), "Cleared tint must restore the stored base albedo")
	assert(light.light_color.is_equal_approx(Color.WHITE) and is_equal_approx(light.light_energy, 1.0))
	Visuals.apply(node, {"tint":[2, 2, 2]})
	assert(mesh.get_active_material(0).albedo_color.is_equal_approx(Color(1, 1, 1)))
	node.free()
	# (c) Block-flag packing used by the People tab, plus its row wiring.
	assert(People.pack_flags(true, false, true, false, false) == 5)
	assert(People.pack_flags(true, true, true, true, true) == 31)
	assert(People.pack_flags(false, false, false, false, false) == 0)
	var unpacked: Dictionary = People.unpack_flags(21)
	assert(unpacked.voice and unpacked.touch and unpacked.complete and not unpacked.text and not unpacked.avatar)
	var people := People.new()
	tree.root.add_child(people)
	people.players([{"id":"self", "name":"Me"}, {"id":"p1", "name":"Pat"}, {"id":"p2", "name":"Sam"}], "self")
	assert(people.rows.size() == 2 and int(people.blocks.p1) == 0)
	people.rows.p2.checks.complete.set_pressed_no_signal(true)
	people._flag_toggled(true, "p2")
	assert(int(people.puts.p2) == 16)
	var list := JSON.stringify({"blocks":[{"target":"p1","flags":7,"voice":true,"text":true,"touch":true,"avatar":false,"complete":false}]}).to_utf8_buffer()
	people._completed(HTTPRequest.RESULT_SUCCESS, 200, PackedStringArray(), list)
	assert(int(people.blocks.p1) == 7 and people.rows.p1.checks.voice.button_pressed and not people.rows.p1.checks.complete.button_pressed)
	people.players([{"id":"p1", "name":"Pat"}], "self")
	assert(people.rows.size() == 1)
	people.free()
	app.free()
	await tree.process_frame
	print("PASS manifest-less snapshots keep the loaded world, visual overrides restore stored bases, and People-tab flags pack exactly")
