extends RefCounted
const Animations = preload("res://scripts/creator_animations.gd")

static func run() -> void:
	var host := Node3D.new()
	var root := Node3D.new()
	root.name = "Model"
	host.add_child(root)
	var foreign := Node3D.new()
	foreign.name = "Foreign"
	host.add_child(foreign)
	var mesh := MeshInstance3D.new()
	mesh.name = "Mesh"
	mesh.mesh = BoxMesh.new()
	root.add_child(mesh)
	var player := AnimationPlayer.new()
	player.name = "Clips"
	player.root_node = NodePath("..")
	root.add_child(player)
	var clip := Animation.new()
	clip.length = 2
	var position := clip.add_track(Animation.TYPE_POSITION_3D)
	clip.track_set_path(position,NodePath("Mesh"))
	clip.position_track_insert_key(position,0,Vector3.ZERO)
	clip.position_track_insert_key(position,2,Vector3(2,0,0))
	var property := clip.add_track(Animation.TYPE_VALUE)
	clip.track_set_path(property,NodePath("Mesh:visible"))
	clip.track_insert_key(property,0,false)
	var method := clip.add_track(Animation.TYPE_METHOD)
	clip.track_set_path(method,NodePath("."))
	clip.track_insert_key(method,0,{"method":"queue_free","args":[]})
	var escape := clip.add_track(Animation.TYPE_POSITION_3D)
	clip.track_set_path(escape,NodePath("../Foreign"))
	clip.position_track_insert_key(escape,0,Vector3(999,0,0))
	var library := AnimationLibrary.new()
	library.add_animation("move",clip)
	player.add_animation_library("",library)
	player.autoplay = "move"
	Animations.normalize(root)
	assert(player.autoplay.is_empty())
	assert(player.get_animation("move").get_track_count() == 1)
	assert(clip.get_track_count() == 4) # Shared source resources are unchanged.
	assert(Animations.apply(root,"world",{"action":"play_animation","target":"Clips","clip":"move","value":1}).is_empty())
	assert(Animations.apply(root,"world",{"action":"seek_animation","target":"Clips","value":1}).is_empty())
	assert(mesh.position.is_equal_approx(Vector3(1,0,0)))
	assert(foreign.position == Vector3.ZERO and mesh.visible)
	assert(not Animations.apply(root,"Mesh",{"action":"play_animation","target":"Clips","clip":"move","value":1}).is_empty())
	assert(not Animations.apply(root,"world",{"action":"play_animation","target":"Clips","clip":"missing","value":1}).is_empty())
	assert(not Animations.apply(root,"world",{"action":"seek_animation","target":"Clips","value":3}).is_empty())
	assert(Animations.apply(root,"world",{"action":"stop_animation","target":"Clips"}).is_empty())
	host.free()
	print("PASS supplied transform-only animation playback/seek/stop, dangerous-track stripping and scope/resource isolation")
