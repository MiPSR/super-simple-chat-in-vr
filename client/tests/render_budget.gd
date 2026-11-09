extends RefCounted
const Budget = preload("res://scripts/render_budget.gd")
const Materials = preload("res://scripts/materials.gd")

static func run() -> void:
	var root := Node3D.new()
	var first := MeshInstance3D.new()
	first.mesh = BoxMesh.new()
	var second := MeshInstance3D.new()
	second.mesh = first.mesh
	root.add_child(first)
	root.add_child(second)
	var image := Image.create(16,16,false,Image.FORMAT_RGBA8)
	var texture := ImageTexture.create_from_image(image)
	var material := StandardMaterial3D.new()
	material.albedo_texture = texture
	first.material_override = material
	second.material_override = material
	Materials.prepare(root,0)
	var reservation := Budget.reserve(root)
	assert(not reservation.has("error"))
	assert(reservation.nodes == 3 and reservation.surfaces == 6)
	assert(reservation.texture_bytes == ceili(16.0 * 16.0 * 16.0 / 3.0))
	var shared := Budget.reserve(root,false)
	assert(not shared.has("error") and shared.texture_bytes == 0)
	var invalid = ClassDB.instantiate("RenderLease")
	assert(not invalid.reserve(-1,0,0,0,0,0).is_empty())
	assert(not invalid.reserve(32769,0,0,0,0,0).is_empty())
	assert(not invalid.reserve(0,0,0,0,4097,0).is_empty())
	assert(not invalid.reserve(0,0,0,0,0,65).is_empty())
	var owned = ClassDB.instantiate("RenderLease")
	assert(owned.reserve(0,0,0,0,0,0).is_empty())
	assert(not owned.reserve(0,0,0,0,0,0).is_empty())
	root.free()
	print("PASS aggregate render reservations, tier-independent pass costs, shared-texture accounting and invalid reservation rejection")
