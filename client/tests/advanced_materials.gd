extends RefCounted
const Materials = preload("res://scripts/materials.gd")
const Styles = preload("res://scripts/material_styles.gd")
const Advanced = preload("res://scripts/advanced_materials.gd")

static func run() -> void:
	var root := Node3D.new()
	var mesh := MeshInstance3D.new()
	mesh.name = "Surface"
	mesh.mesh = BoxMesh.new()
	root.add_child(mesh)
	Materials.prepare(root,2)
	var style := {"albedo":[0.3,0.4,0.5],"outline_width":0.005,"outline_color":[0,0,0],"rim":0.4,"rim_tint":0.7,"matcap":"pearl","matcap_strength":0.2,"specular":0.6}
	assert(Styles.apply(root,{"Surface":style}).is_empty())
	var material := mesh.get_active_material(0) as StandardMaterial3D
	assert(material.rim_enabled and is_equal_approx(material.rim,0.4))
	assert(is_equal_approx(material.metallic_specular,0.6))
	var outline := material.next_pass as StandardMaterial3D
	assert(outline != null and outline.grow and is_equal_approx(outline.grow_amount,0.005))
	var matcap := outline.next_pass as ShaderMaterial
	assert(matcap != null and matcap.shader == Advanced.MatcapShader)
	assert(matcap.get_shader_parameter("matcap_texture") == Advanced.matcap("pearl"))
	assert(Advanced.matcap("metal") != Advanced.matcap("pearl"))
	for tier in [0,1]:
		Materials.apply(root,tier)
		material = mesh.get_active_material(0) as StandardMaterial3D
		assert(not material.rim_enabled and material.next_pass == null)
		assert(material.albedo_color.is_equal_approx(Color(0.3,0.4,0.5)))
	Materials.apply(root,2)
	material = mesh.get_active_material(0) as StandardMaterial3D
	assert(material.rim_enabled and material.next_pass != null)
	assert(not Styles.apply(root,{"Missing":style}).is_empty())
	root.free()
	print("PASS fixed advanced-material schema, cached built-in matcaps, bounded outline/rim passes and tier persistence")
