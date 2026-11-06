extends RefCounted
## One constrained material family applied to both worlds and avatars.
static func apply(root: Node, tier: int, brightness: float = 1.0) -> void:
	if root is MeshInstance3D and root.mesh:
		var originals: Array = root.get_meta("ssc_original_materials", [])
		if originals.size() != root.mesh.get_surface_count():
			originals.clear()
			for i in root.mesh.get_surface_count():
				originals.append(root.get_active_material(i))
			root.set_meta("ssc_original_materials", originals)
		for i in root.mesh.get_surface_count():
			var original: Material = originals[i]
			var mat := StandardMaterial3D.new()
			if original is StandardMaterial3D:
				mat.albedo_color = original.albedo_color
				mat.albedo_texture = original.albedo_texture
				mat.transparency = original.transparency
				mat.cull_mode = original.cull_mode
				mat.emission_enabled = tier == 2 and original.emission_enabled
				mat.emission = original.emission
				mat.emission_texture = original.emission_texture
			elif original is ShaderMaterial:
				var texture = original.get_shader_parameter("_MainTex")
				var color = original.get_shader_parameter("_Color")
				if texture is Texture2D:
					mat.albedo_texture = texture
				if color is Color:
					mat.albedo_color = color
			mat.albedo_color *= Color(brightness, brightness, brightness, 1)
			mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED if tier == 0 else BaseMaterial3D.SHADING_MODE_PER_PIXEL
			mat.diffuse_mode = BaseMaterial3D.DIFFUSE_TOON if tier == 2 else BaseMaterial3D.DIFFUSE_LAMBERT
			mat.specular_mode = BaseMaterial3D.SPECULAR_TOON if tier == 2 else BaseMaterial3D.SPECULAR_DISABLED
			mat.roughness = 0.8
			root.set_surface_override_material(i, mat)
		root.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF if tier == 0 else GeometryInstance3D.SHADOW_CASTING_SETTING_ON
	for child in root.get_children():
		apply(child, tier, brightness)
