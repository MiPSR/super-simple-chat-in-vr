extends RefCounted
const Materials = preload("res://scripts/materials.gd")

static func run() -> void:
	var root := Node3D.new()
	var lights: Array[OmniLight3D] = []
	for index in 6:
		var light := OmniLight3D.new()
		light.shadow_enabled = true
		light.light_energy = 1000
		light.omni_range = 1000
		root.add_child(light)
		lights.append(light)
	Materials.prepare(root,2)
	assert(lights.filter(func(light): return light.shadow_enabled).size() == Materials.SHADOW_LIGHT_LIMIT)
	assert(is_equal_approx(lights[0].light_energy,8) and is_equal_approx(lights[0].omni_range,100))
	assert(not lights[4].shadow_enabled and not lights[5].shadow_enabled)
	lights[0].visible = false
	Materials.refresh_shadows()
	assert(not lights[0].shadow_enabled and lights[4].shadow_enabled)
	Materials.apply(root,0)
	assert(lights.all(func(light): return not light.shadow_enabled))
	Materials.apply(root,2)
	assert(lights.filter(func(light): return light.shadow_enabled).size() == Materials.SHADOW_LIGHT_LIMIT)
	root.free()
	Materials.refresh_shadows()
	assert(Materials.shadow_lights.is_empty())
	print("PASS aggregate four-light shadow limit, waiting-light promotion, tier-zero suppression and authored-light bounds")
