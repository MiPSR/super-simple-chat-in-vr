extends RefCounted
const Materials = preload("res://scripts/materials.gd")

static func run() -> void:
	var mesh := MeshInstance3D.new()
	mesh.mesh = BoxMesh.new()
	var original := StandardMaterial3D.new()
	original.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	original.albedo_color = Color(0.2,0.3,0.4,0.35)
	original.roughness = 0.27
	original.uv1_scale = Vector3(2,3,1)
	original.uv1_offset = Vector3(0.1,0.2,0)
	mesh.material_override = original
	Materials.prepare(mesh,0)
	var material := mesh.get_active_material(0) as StandardMaterial3D
	assert(material.transparency == BaseMaterial3D.TRANSPARENCY_ALPHA_HASH)
	assert(is_equal_approx(material.albedo_color.a,0.35))
	# Godot omits unused unlit parameters while duplicating a material; retain
	# the authored value in the source for restoration in lit tiers.
	assert(is_equal_approx(mesh.get_meta("ssc_original_materials")[0].roughness,0.27))
	assert(material.uv1_scale.is_equal_approx(Vector3(2,3,1)) and material.uv1_offset.is_equal_approx(Vector3(0.1,0.2,0)))
	Materials.apply(mesh,2)
	material = mesh.get_active_material(0) as StandardMaterial3D
	assert(material.transparency == BaseMaterial3D.TRANSPARENCY_ALPHA)
	assert(is_equal_approx(material.roughness,0.27))
	assert(original.transparency == BaseMaterial3D.TRANSPARENCY_ALPHA)
	mesh.free()
	mesh = MeshInstance3D.new()
	mesh.mesh = BoxMesh.new()
	original = StandardMaterial3D.new()
	original.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA_SCISSOR
	original.alpha_scissor_threshold = 0.2
	mesh.material_override = original
	Materials.prepare(mesh,0)
	for tier in [0,1,2]:
		Materials.apply(mesh,tier)
		material = mesh.get_active_material(0) as StandardMaterial3D
		assert(material.transparency == BaseMaterial3D.TRANSPARENCY_ALPHA_SCISSOR)
		assert(is_equal_approx(material.alpha_scissor_threshold,0.2))
	mesh.free()
	assert(Materials._color(Vector4(0.2,0.3,0.4,0.5),Color.WHITE).is_equal_approx(Color(0.2,0.3,0.4,0.5)))
	assert(Materials._color(Color(1000000,-1,1,2),Color.WHITE).is_equal_approx(Color(1,0,1,1)))
	assert(Materials._color(Color(NAN,0,0,1),Color.WHITE) == Color.WHITE)
	assert(Materials._color(Plane(Vector3(0.2,0.3,0.4),0.5),Color.WHITE).is_equal_approx(Color(0.2,0.3,0.4,0.5)))
	for variant in ["mtoon_trans","mtoon_cutout"]:
		mesh = MeshInstance3D.new()
		mesh.mesh = BoxMesh.new()
		var imported := ShaderMaterial.new()
		imported.shader = load("res://addons/Godot-MToon-Shader/%s.gdshader" % variant)
		assert(imported.shader != null)
		imported.set_shader_parameter("_Color",Color(0.2,0.3,0.4,0.35))
		imported.set_shader_parameter("_Cutoff",0.2)
		imported.set_shader_parameter("_MainTex_ST",Vector4(2,3,0.1,0.2))
		imported.set_shader_parameter("_EmissionColor",Color(0.2,0.1,0.3))
		imported.set_shader_parameter("_EmissionMultiplier",2.0)
		mesh.material_override = imported
		Materials.prepare(mesh,0)
		material = mesh.get_active_material(0) as StandardMaterial3D
		assert(material.albedo_color.is_equal_approx(Color(0.2,0.3,0.4,0.35)))
		assert(material.transparency == (BaseMaterial3D.TRANSPARENCY_ALPHA_HASH if variant == "mtoon_trans" else BaseMaterial3D.TRANSPARENCY_ALPHA_SCISSOR))
		assert(material.uv1_scale.is_equal_approx(Vector3(2,3,1)) and material.uv1_offset.is_equal_approx(Vector3(0.1,0.2,0)))
		assert(not material.emission_enabled)
		Materials.apply(mesh,2)
		material = mesh.get_active_material(0) as StandardMaterial3D
		assert(material.emission_enabled and is_equal_approx(material.emission_energy_multiplier,2.0))
		mesh.free()
	print("PASS ultra-tier alpha dithering, authored alpha/cutoff preservation and vector shader-color normalization")
