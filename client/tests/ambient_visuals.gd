extends RefCounted
const Ambient = preload("res://scripts/ambient_visuals.gd")
const Materials = preload("res://scripts/materials.gd")
const Visuals = preload("res://scripts/creator_visuals.gd")
const Runtime = preload("res://scripts/creator_runtime.gd")

class Host extends Node:
	var calls := 0
	func creator_ambient(_color: Array, _brightness: float) -> void:
		calls += 1

static func run() -> void:
	var ambient := Ambient.new()
	ambient.clear()
	var root := Node3D.new()
	var mesh := MeshInstance3D.new()
	mesh.name = "Panel"
	mesh.mesh = BoxMesh.new()
	root.add_child(mesh)
	Materials.prepare(root,0)
	assert(Visuals.apply(root,"world",{"action":"material","target":"Panel","property":"albedo","value":[0.8,0.4,0.2]}).is_empty())
	ambient.request([root],Color(0.5,1,1),0.5)
	ambient.advance()
	var material := mesh.get_active_material(0) as StandardMaterial3D
	assert(material.albedo_color.is_equal_approx(Color(0.2,0.2,0.1)))
	assert(Visuals.apply(root,"world",{"action":"material","target":"Panel","property":"roughness","value":0.3}).is_empty())
	material = mesh.get_active_material(0) as StandardMaterial3D
	assert(material.albedo_color.is_equal_approx(Color(0.2,0.2,0.1)))
	Materials.apply(root,2)
	material = mesh.get_active_material(0) as StandardMaterial3D
	assert(material.albedo_color.is_equal_approx(Color(0.2,0.2,0.1)))
	ambient.request([root],Color.WHITE,0)
	ambient.advance()
	assert(material.albedo_color.is_equal_approx(Color.BLACK))
	ambient.request([root],Color.WHITE,1)
	ambient.advance()
	assert(material.albedo_color.is_equal_approx(Color(0.8,0.4,0.2)))
	# Incremental walks yield and do not restart/starve on every new request.
	for index in 600:
		root.add_child(Node3D.new())
	ambient.request([root],Color.WHITE,0.5)
	ambient.advance()
	assert(not ambient.stack.is_empty())
	for index in 30:
		ambient.request([root],Color.WHITE,0.5)
		ambient.advance()
	ambient.request([root],Color.WHITE,1)
	for index in 20:
		ambient.advance()
	assert(ambient.stack.is_empty() and ambient.roots.is_empty())
	# Freed roots are weak references, never kept alive by presentation work.
	ambient.request([root],Color.WHITE,0.5)
	root.free()
	ambient.advance()
	assert(ambient.stack.is_empty())
	var host := Host.new()
	var scope := Node3D.new()
	scope.set_meta("ssc_visual_host",host)
	var runtime := Runtime.new()
	runtime.scope = scope
	var errors: Array[String] = []
	runtime.script_error.connect(func(message: String): errors.append(message))
	var command := {"action":"ambient","color":[1,1,1],"brightness":0.5}
	runtime._apply(command,"avatar")
	runtime._apply(command,"Panel")
	assert(host.calls == 0 and errors.size() == 2)
	runtime._apply(command,"world")
	assert(host.calls == 1)
	runtime.free()
	scope.free()
	host.free()
	ambient.clear()
	assert(Materials.ambient_tint == Color.WHITE)
	print("PASS incremental global ambient tint/darkness, untinted base preservation, tier persistence and attachment isolation")
