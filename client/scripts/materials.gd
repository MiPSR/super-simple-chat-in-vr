extends RefCounted
## One constrained material family applied to both worlds and avatars.
static var ambient_tint := Color.WHITE
static var shadow_lights: Array[WeakRef] = []
const SHADOW_LIGHT_LIMIT := 4
static func prepare(root: Node, tier: int) -> void:
	# Uploaded glTF extras must not impersonate host-owned renderer metadata.
	_clear_host_metadata(root)
	apply(root,tier)
	_clear_mesh_materials(root)

static func _source(original: Material) -> StandardMaterial3D:
	# Retain only the standardized family's source data, never arbitrary shader
	# state/next-pass chains and their otherwise-unused uploaded textures.
	var source := StandardMaterial3D.new()
	source.roughness = 0.8
	if original is StandardMaterial3D:
		source.albedo_color = _color(original.albedo_color,Color.WHITE)
		source.albedo_texture = original.albedo_texture
		source.transparency = original.transparency
		source.alpha_scissor_threshold = original.alpha_scissor_threshold
		source.uv1_scale = original.uv1_scale
		source.uv1_offset = original.uv1_offset
		source.cull_mode = original.cull_mode
		source.emission_enabled = original.emission_enabled
		source.emission = _color(original.emission,Color.BLACK)
		source.emission_texture = original.emission_texture
		source.emission_energy_multiplier = clampf(original.emission_energy_multiplier,0,8)
		source.metallic = original.metallic
		source.roughness = original.roughness
		source.metallic_specular = original.metallic_specular
		source.normal_enabled = original.normal_enabled
		source.normal_texture = original.normal_texture
		source.normal_scale = clampf(original.normal_scale,-4,4)
	elif original is ShaderMaterial:
		var texture = original.get_shader_parameter("_MainTex")
		var color = original.get_shader_parameter("_Color")
		if texture is Texture2D:
			source.albedo_texture = texture
		source.albedo_color = _color(color,Color.WHITE)
		# Interpret only the pinned importer's fixed MToon uniforms/variants.
		# No supplied shader or next-pass resource survives normalization.
		if original.shader and original.shader.resource_path.begins_with("res://addons/Godot-MToon-Shader/"):
			var variant: String = original.shader.resource_path.get_file().get_basename()
			if variant.begins_with("mtoon_trans"):
				source.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
			elif variant.begins_with("mtoon_cutout"):
				source.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA_SCISSOR
				var cutoff = original.get_shader_parameter("_Cutoff")
				if cutoff is float or cutoff is int:
					source.alpha_scissor_threshold = clampf(float(cutoff),0,1)
			var normal = original.get_shader_parameter("_BumpMap")
			if normal is Texture2D:
				source.normal_texture = normal
				source.normal_enabled = true
				source.normal_scale = _number(original.get_shader_parameter("_BumpScale"),1,0,4)
			var emission = original.get_shader_parameter("_EmissionMap")
			if emission is Texture2D:
				source.emission_texture = emission
			source.emission = _color(original.get_shader_parameter("_EmissionColor"),Color.BLACK)
			source.emission_energy_multiplier = _number(original.get_shader_parameter("_EmissionMultiplier"),1,0,8)
			source.emission_enabled = source.emission_texture != null or source.emission.r > 0 or source.emission.g > 0 or source.emission.b > 0
			var uv = original.get_shader_parameter("_MainTex_ST")
			if uv is Vector4:
				source.uv1_scale = Vector3(uv.x,uv.y,1)
				source.uv1_offset = Vector3(uv.z,uv.w,0)
			elif uv is Plane:
				source.uv1_scale = Vector3(uv.x,uv.y,1)
				source.uv1_offset = Vector3(uv.z,uv.d,0)
	return source

static func _number(value: Variant, fallback: float, minimum: float, maximum: float) -> float:
	return clampf(float(value),minimum,maximum) if (value is float or value is int) and is_finite(float(value)) else fallback

static func _color(value: Variant, fallback: Color) -> Color:
	var color := fallback
	if value is Color:
		color = value
	elif value is Vector4:
		color = Color(value.x,value.y,value.z,value.w)
	elif value is Plane:
		color = Color(value.x,value.y,value.z,value.d)
	if not is_finite(color.r) or not is_finite(color.g) or not is_finite(color.b) or not is_finite(color.a):
		return fallback
	return color.clamp(Color(0,0,0,0),Color(1,1,1,1))

static func _clear_mesh_materials(root: Node) -> void:
	# All instances captured their source before this second traversal. Clearing
	# shared mesh slots therefore cannot erase a sibling's authored source.
	if root is MeshInstance3D and root.mesh:
		if root.mesh is PrimitiveMesh:
			if root.mesh.material != null:
				root.mesh.material = null
		elif root.mesh is ArrayMesh:
			for index in root.mesh.get_surface_count():
				if root.mesh.surface_get_material(index) != null:
					root.mesh.surface_set_material(index,null)
	for child in root.get_children():
		_clear_mesh_materials(child)

static func _clear_host_metadata(root: Node) -> void:
	for key in root.get_meta_list():
		if str(key).begins_with("ssc_"):
			root.remove_meta(key)
	for child in root.get_children():
		_clear_host_metadata(child)

static func apply(root: Node, tier: int, brightness: float = 1.0) -> void:
	root.set_meta("ssc_quality",tier)
	if root is Light3D:
		root.light_energy = clampf(root.light_energy,0,8)
		if root is OmniLight3D:
			root.omni_range = clampf(root.omni_range,0.01,100)
		elif root is SpotLight3D:
			root.spot_range = clampf(root.spot_range,0.01,100)
		elif root is DirectionalLight3D:
			root.directional_shadow_max_distance = minf(root.directional_shadow_max_distance,100)
		if not root.has_meta("ssc_authored_shadow"):
			root.set_meta("ssc_authored_shadow",root.shadow_enabled)
		apply_light_overrides(root)
	if root is MeshInstance3D and root.mesh:
		var originals: Array = root.get_meta("ssc_original_materials", [])
		if originals.size() != root.mesh.get_surface_count():
			originals.clear()
			for i in root.mesh.get_surface_count():
				originals.append(_source(root.get_active_material(i)))
			root.set_meta("ssc_original_materials", originals)
		root.material_override = null
		for i in root.mesh.get_surface_count():
			var original: Material = originals[i]
			var mat := StandardMaterial3D.new()
			if original is StandardMaterial3D:
				mat.albedo_color = original.albedo_color
				mat.albedo_texture = original.albedo_texture
				mat.transparency = original.transparency
				mat.alpha_scissor_threshold = original.alpha_scissor_threshold
				mat.uv1_scale = original.uv1_scale
				mat.uv1_offset = original.uv1_offset
				mat.cull_mode = original.cull_mode
				mat.emission_enabled = tier == 2 and original.emission_enabled
				mat.emission = original.emission
				mat.emission_texture = original.emission_texture
				mat.emission_energy_multiplier = original.emission_energy_multiplier
				mat.metallic = original.metallic
				mat.metallic_specular = original.metallic_specular
				mat.normal_enabled = tier == 2 and original.normal_enabled
				mat.normal_texture = original.normal_texture
				mat.normal_scale = original.normal_scale
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
			mat.specular_mode = BaseMaterial3D.SPECULAR_TOON if tier == 2 else BaseMaterial3D.SPECULAR_SCHLICK_GGX
			if tier == 0:
				mat.specular_mode = BaseMaterial3D.SPECULAR_DISABLED
				if mat.transparency in [BaseMaterial3D.TRANSPARENCY_ALPHA,BaseMaterial3D.TRANSPARENCY_ALPHA_DEPTH_PRE_PASS]:
					mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA_HASH
			mat.roughness = original.roughness if original is StandardMaterial3D else 0.8
			root.set_surface_override_material(i, mat)
		apply_overrides(root)
		apply_tint(root)
		root.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF if tier == 0 else GeometryInstance3D.SHADOW_CASTING_SETTING_ON
	for child in root.get_children():
		apply(child, tier, brightness)

static func apply_overrides(mesh: MeshInstance3D) -> void:
	if not mesh.mesh:
		return
	var values: Dictionary = mesh.get_meta("ssc_creator_material", {})
	if values.is_empty():
		return
	var tier := int(mesh.get_meta("ssc_quality",1))
	for index in mesh.mesh.get_surface_count():
		var material := mesh.get_surface_override_material(index) as StandardMaterial3D
		if not material:
			continue
		# PackedScene instances may share surface overrides. Never mutate that
		# shared resource, even if it was normalized before packing the prefab.
		material = material.duplicate() as StandardMaterial3D
		mesh.set_surface_override_material(index,material)
		if material.has_meta("ssc_base_albedo"):
			material.albedo_color = material.get_meta("ssc_base_albedo")
		if values.has("albedo"):
			var color: Array = values.albedo
			material.albedo_color = Color(float(color[0]),float(color[1]),float(color[2]),material.albedo_color.a)
		if values.has("roughness"): material.roughness = float(values.roughness)
		if values.has("metallic"): material.metallic = float(values.metallic)
		if values.has("emission"):
			var color: Array = values.emission
			material.emission = Color(float(color[0]),float(color[1]),float(color[2]))
		if values.has("emission_energy"): material.emission_energy_multiplier = float(values.emission_energy)
		if values.has("emission") or values.has("emission_energy"):
			# Preserve scripted signal colors even in the unlit approximation;
			# lower tiers omit the extra authored emission texture, not the state.
			material.emission_enabled = true
			if tier < 2: material.emission_texture = null
		material.set_meta("ssc_base_albedo",material.albedo_color)
		preload("res://scripts/advanced_materials.gd").apply(material,values,tier)
	apply_tint(mesh)

static func apply_tint(mesh: MeshInstance3D) -> void:
	if not mesh.mesh:
		return
	for index in mesh.mesh.get_surface_count():
		var material := mesh.get_surface_override_material(index) as StandardMaterial3D
		if not material:
			continue
		if int(material.get_meta("ssc_tint_owner",0)) != mesh.get_instance_id():
			material = material.duplicate() as StandardMaterial3D
			mesh.set_surface_override_material(index,material)
			material.set_meta("ssc_tint_owner",mesh.get_instance_id())
		if not material.has_meta("ssc_base_albedo"):
			material.set_meta("ssc_base_albedo",material.albedo_color)
		var base: Color = material.get_meta("ssc_base_albedo")
		material.albedo_color = base * ambient_tint

static func apply_light_overrides(light: Light3D) -> void:
	var values: Dictionary = light.get_meta("ssc_creator_light",{})
	if values.has("color"):
		var color: Array = values.color
		light.light_color = Color(float(color[0]),float(color[1]),float(color[2]))
	if values.has("energy"): light.light_energy = float(values.energy)
	if values.has("enabled"): light.visible = bool(values.enabled)
	if values.has("range"):
		if light is OmniLight3D: light.omni_range = float(values.range)
		elif light is SpotLight3D: light.spot_range = float(values.range)
	if values.has("shadow"):
		light.set_meta("ssc_creator_shadow",bool(values.shadow))
	if not shadow_lights.any(func(reference): return reference.get_ref() == light):
		shadow_lights.append(weakref(light))
	refresh_shadows()

static func refresh_shadows() -> void:
	var retained: Array[WeakRef] = []
	var allocated := 0
	for reference in shadow_lights:
		var light = reference.get_ref()
		if not is_instance_valid(light) or light.is_queued_for_deletion():
			continue
		retained.append(reference)
		var requested: bool = bool(light.get_meta("ssc_creator_shadow",light.get_meta("ssc_authored_shadow",false)))
		var enabled: bool = light.visible and int(light.get_meta("ssc_quality",1)) > 0 and requested and allocated < SHADOW_LIGHT_LIMIT
		light.shadow_enabled = enabled
		if enabled:
			allocated += 1
	shadow_lights = retained
