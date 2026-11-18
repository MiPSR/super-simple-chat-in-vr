extends RefCounted
## Server-authoritative snapshot "visual" overrides applied per entity node.
## Tint multiplies stored base albedo colors, light rewrites Light3D children,
## animation plays/stops the named AnimationPlayer track set. Absent fields
## restore the stored bases.
const Materials = preload("res://scripts/materials.gd")

static func apply(entity: Node3D, entry: Dictionary) -> void:
	if not is_instance_valid(entity):
		return
	_tint(entity, entry.get("tint"))
	_lights(entity, entry.get("light"))
	_animation(entity, entry)

static func _tint(entity: Node3D, tint: Variant) -> void:
	if tint is Array and tint.size() >= 3:
		var color := Color(float(tint[0]), float(tint[1]), float(tint[2]))
		if is_finite(color.r) and is_finite(color.g) and is_finite(color.b):
			entity.set_meta("ssc_visual_tint", color.clamp(Color(0, 0, 0), Color(2, 2, 2)))
			_retint(entity)
	elif entity.has_meta("ssc_visual_tint"):
		entity.remove_meta("ssc_visual_tint")
		_retint(entity)

static func _retint(entity: Node3D) -> void:
	var meshes: Array = entity.find_children("*", "MeshInstance3D", true, false)
	if entity is MeshInstance3D:
		meshes.append(entity)
	for mesh in meshes:
		Materials.apply_tint(mesh)

static func _lights(entity: Node3D, light: Variant) -> void:
	var lamps: Array = entity.find_children("*", "Light3D", true, false)
	if entity is Light3D:
		lamps.append(entity)
	if lamps.is_empty():
		return
	if light is Array and light.size() >= 4:
		var color := Color(float(light[0]), float(light[1]), float(light[2]))
		var energy := float(light[3])
		if not (is_finite(color.r) and is_finite(color.g) and is_finite(color.b) and is_finite(energy)):
			return
		for lamp in lamps:
			var node := lamp as Light3D
			if not node.has_meta("ssc_visual_light"):
				node.set_meta("ssc_visual_light", {"color": node.light_color, "energy": node.light_energy})
			node.light_color = color
			node.light_energy = energy
	else:
		for lamp in lamps:
			var node := lamp as Light3D
			if node.has_meta("ssc_visual_light"):
				var base: Dictionary = node.get_meta("ssc_visual_light")
				node.light_color = base.color
				node.light_energy = float(base.energy)
				node.remove_meta("ssc_visual_light")

static func _animation(entity: Node3D, entry: Dictionary) -> void:
	var players: Array = entity.find_children("*", "AnimationPlayer", true, false)
	if players.is_empty():
		return
	var name := str(entry.get("animation", ""))
	if name.is_empty():
		for player in players:
			if player.has_meta("ssc_visual_animation"):
				player.stop()
				player.remove_meta("ssc_visual_animation")
		return
	var playing := bool(entry.get("animation_playing", false))
	for player in players:
		if not player.has_animation(name):
			continue
		player.set_meta("ssc_visual_animation", name)
		if playing:
			# Re-applied snapshots must not restart an already running track.
			if player.current_animation != name or not player.is_playing():
				player.play(name)
		elif player.is_playing():
			player.stop()
