extends RefCounted
const Runtime = preload("res://scripts/vrm_runtime.gd")
const Materials = preload("res://scripts/materials.gd")

static func run() -> void:
	var runtime := Runtime.new()
	var extension := preload("res://scripts/texture_import.gd").new()
	var small := Image.create(1, 2, false, Image.FORMAT_RGB8)
	var reduced_state := GLTFState.new()
	reduced_state.set_additional_data(&"ssc/texture_reduction", 3)
	for mime in ["image/png", "image/jpeg", "image/webp"]:
		var data: PackedByteArray
		match mime:
			"image/png": data = small.save_png_to_buffer()
			"image/jpeg": data = small.save_jpg_to_buffer()
			"image/webp": data = small.save_webp_to_buffer()
		var output := Image.new()
		assert(extension._parse_image_data(reduced_state, data, mime, output) == OK)
		assert(output.get_width() == 1 and output.get_height() == 1)
	var image := Image.create(32, 16, false, Image.FORMAT_RGBA8)
	image.fill(Color(0.2, 0.6, 0.9, 0.5))
	var png := image.save_png_to_buffer()
	var binary := PackedFloat32Array([0,0,0, 1,0,0, 0,1,0]).to_byte_array()
	var offset := binary.size()
	binary.append_array(png)
	while binary.size() % 4 != 0:
		binary.append(0)
	var doc := {"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
		"buffers":[{"byteLength":binary.size()}], "bufferViews":[{"buffer":0,"byteLength":offset},{"buffer":0,"byteOffset":offset,"byteLength":png.size()}],
		"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],
		"images":[{"bufferView":1,"mimeType":"image/png"}],"textures":[{"source":0}],
		"materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}},"alphaMode":"BLEND"}],
		"meshes":[{"primitives":[{"attributes":{"POSITION":0},"material":0}]}]}
	var json := JSON.stringify(doc).to_utf8_buffer()
	while json.size() % 4 != 0:
		json.append(32)
	var glb := PackedByteArray()
	glb.append_array(PackedInt32Array([0x46546c67,2,28+json.size()+binary.size(),json.size(),0x4e4f534a]).to_byte_array())
	glb.append_array(json)
	glb.append_array(PackedInt32Array([binary.size(),0x004e4942]).to_byte_array())
	glb.append_array(binary)
	for reduction in [0,1,2,3,0]:
		var document := GLTFDocument.new()
		var state := GLTFState.new()
		state.set_additional_data(&"ssc/texture_reduction", reduction)
		assert(document.append_from_buffer(glb, "", state) == OK)
		var texture: Texture2D = state.get_images()[0]
		assert(texture.get_width() == 32 >> reduction and texture.get_height() == 16 >> reduction, "Importer did not reduce actual texture dimensions")
		var root: Node3D = document.generate_scene(state)
		assert(root != null)
		Engine.get_main_loop().root.add_child(root)
		var mesh: MeshInstance3D = root.find_children("*", "MeshInstance3D", true, false)[0]
		for tier in [0,1,2]:
			Materials.apply(root, tier)
			var material: StandardMaterial3D = mesh.get_active_material(0)
			assert(material.albedo_texture.get_width() == 32 >> reduction)
			assert(material.albedo_texture.get_height() == 16 >> reduction)
			assert(mesh.get_meta("ssc_original_materials")[0].albedo_texture.get_width() == 32 >> reduction, "Original material retained a larger texture")
		mesh.set_surface_override_material(0, null)
		mesh.mesh = null
		root.free()
	runtime.close()
	print("PASS import-time texture allocation dimensions, all material tiers, and independent per-import policies")
