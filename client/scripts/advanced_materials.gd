extends RefCounted
## Project-owned extra passes only. Uploads cannot provide shader source.
const MatcapShader = preload("res://shaders/matcap.gdshader")
static var matcaps: Dictionary = {}

static func matcap(name: String) -> Texture2D:
	if matcaps.has(name):
		return matcaps[name]
	var image := Image.create(64,64,false,Image.FORMAT_RGBA8)
	for y in 64:
		for x in 64:
			var normal := Vector2((x + 0.5) / 32.0 - 1.0,1.0 - (y + 0.5) / 32.0)
			var z := sqrt(maxf(0,1.0 - normal.length_squared()))
			var shine := pow(maxf(0,normal.dot(Vector2(-0.4,0.4)) + z * 0.82),18 if name == "metal" else 5)
			var base := Color(0.12,0.15,0.2) if name == "metal" else Color(0.2,0.14,0.22)
			image.set_pixel(x,y,base * z + Color(shine,shine,shine,1))
	image.generate_mipmaps()
	var texture := ImageTexture.create_from_image(image)
	matcaps[name] = texture
	return texture

static func apply(material: StandardMaterial3D, values: Dictionary, tier: int) -> void:
	material.next_pass = null
	material.rim_enabled = tier == 2 and float(values.get("rim",0)) > 0
	material.rim = float(values.get("rim",0))
	material.rim_tint = float(values.get("rim_tint",0.5))
	if values.has("specular"):
		material.metallic_specular = float(values.specular)
	if tier != 2:
		return
	var outline: StandardMaterial3D
	if float(values.get("outline_width",0)) > 0:
		outline = StandardMaterial3D.new()
		outline.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		outline.cull_mode = BaseMaterial3D.CULL_FRONT
		outline.grow_enabled = true
		outline.grow_amount = float(values.outline_width)
		var color: Array = values.get("outline_color",[0,0,0])
		outline.albedo_color = Color(float(color[0]),float(color[1]),float(color[2]),material.albedo_color.a)
		outline.transparency = material.transparency
		material.next_pass = outline
	var preset := str(values.get("matcap","none"))
	if preset not in ["pearl","metal"] or float(values.get("matcap_strength",0.25)) <= 0:
		return
	var overlay := ShaderMaterial.new()
	overlay.shader = MatcapShader
	overlay.set_shader_parameter("matcap_texture",matcap(preset))
	overlay.set_shader_parameter("albedo_texture",material.albedo_texture)
	overlay.set_shader_parameter("base_alpha",material.albedo_color.a)
	overlay.set_shader_parameter("strength",float(values.get("matcap_strength",0.25)))
	if outline:
		outline.next_pass = overlay
	else:
		material.next_pass = overlay
