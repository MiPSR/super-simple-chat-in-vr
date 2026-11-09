extends Node
## Named source zones select ordinary Godot buses; room size never sets wetness.
var zones: Array[Dictionary] = []
var bus_names: Array[StringName] = []
var effect_ceiling := 0.9
var dirty: Dictionary = {}
var update_cursor := 0
var realistic := false
var listener_position := Vector3.ZERO
var space: World3D
var sources: Dictionary = {}
var source_cursor := 0
var noise_player: AudioStreamPlayer
var noise_rng := RandomNumberGenerator.new()

func configure(specs: Array, ceiling: float, realistic_mode: bool = false) -> void:
	clear()
	realistic = realistic_mode
	effect_ceiling = clampf(ceiling,0,1)
	for spec in specs:
		var name := StringName("SSC zone " + str(get_instance_id()) + " " + str(spec.id))
		var index := AudioServer.bus_count
		AudioServer.add_bus()
		AudioServer.set_bus_name(index, name)
		AudioServer.set_bus_send(index, &"Master")
		bus_names.append(name)
		var effects: Dictionary = spec.get("effects", {})
		_apply_effects(index,effects)
		var p: Array = spec.position
		var s: Array = spec.size
		var size := Vector3(float(s[0]), float(s[1]), float(s[2]))
		var center := Vector3(float(p[0]), float(p[1]), float(p[2]))
		zones.append({"id":str(spec.id),"bounds":AABB(center - size / 2.0, size), "bus":name,"effects":effects.duplicate(true)})

func _apply_effects(index: int, effects: Dictionary) -> void:
	for effect in range(AudioServer.get_bus_effect_count(index) - 1,-1,-1):
		AudioServer.remove_bus_effect(index,effect)
	var reverb_amount := clampf(float(effects.get("reverb",0)),0,effect_ceiling)
	if reverb_amount > 0:
		var reverb := AudioEffectReverb.new()
		reverb.wet = reverb_amount
		reverb.dry = 1.0 - reverb_amount
		AudioServer.add_bus_effect(index,reverb)
	var echo_amount := clampf(float(effects.get("echo",0)),0,effect_ceiling)
	if echo_amount > 0:
		var delay := AudioEffectDelay.new()
		delay.dry = 1.0 - echo_amount * 0.5
		delay.set_tap1_active(true)
		delay.set_tap1_delay_ms(180)
		delay.set_tap1_level_db(linear_to_db(echo_amount))
		delay.set_feedback_active(true)
		delay.set_feedback_delay_ms(180)
		delay.set_feedback_level_db(linear_to_db(echo_amount * 0.6))
		AudioServer.add_bus_effect(index,delay)
	var distortion_amount := clampf(float(effects.get("distortion",0)),0,effect_ceiling)
	if distortion_amount > 0:
		var distortion := AudioEffectDistortion.new()
		distortion.drive = distortion_amount
		distortion.post_gain = -6.0 * distortion_amount
		AudioServer.add_bus_effect(index,distortion)
	if effect_ceiling > 0 and bool(effects.get("compression",false)):
		AudioServer.add_bus_effect(index,AudioEffectCompressor.new())
	AudioServer.add_bus_effect(index,AudioEffectHardLimiter.new())

func creator_property(id: String, property: String, value: Variant) -> String:
	if property not in ["reverb","echo","distortion","compression","noise"]:
		return "Unsupported acoustic zone property"
	for zone in zones:
		if str(zone.id) == id:
			if zone.effects.get(property,false if property == "compression" else 0.0) == value:
				return ""
			zone.effects[property] = value
			dirty[id] = true
			return ""
	return "Unknown acoustic zone"

func _process(_delta: float) -> void:
	_update_noise()
	# Coalesce writes and rotate fairly: at most two effect chains per frame.
	if dirty.is_empty() or zones.is_empty():
		return
	var changed := 0
	for offset in zones.size():
		var current := (update_cursor + offset) % zones.size()
		var zone: Dictionary = zones[current]
		if dirty.has(str(zone.id)):
			var index := AudioServer.get_bus_index(zone.bus)
			if index >= 0:
				_apply_effects(index,zone.effects)
			dirty.erase(str(zone.id))
			changed += 1
			if changed >= 2:
				update_cursor = (current + 1) % zones.size()
				break

func _physics_process(_delta: float) -> void:
	_advance_occlusion()

func listener(position: Vector3, world: World3D) -> void:
	listener_position = position
	space = world

func source_gain(player: AudioStreamPlayer3D, dry: bool = false) -> float:
	var id := player.get_instance_id()
	if dry or not realistic:
		_restore_source(player)
		if sources.has(id):
			sources[id].dry = dry
		return 1.0
	if not sources.has(id):
		if sources.size() >= 512:
			return 1.0
		sources[id] = {"node":weakref(player),"cutoff":player.attenuation_filter_cutoff_hz,"filter_db":player.attenuation_filter_db,"next":0,"dry":false}
	sources[id].dry = false
	return float(player.get_meta("ssc_occlusion_gain",1.0))

func _restore_source(player: AudioStreamPlayer3D) -> void:
	var id := player.get_instance_id()
	if sources.has(id):
		player.attenuation_filter_cutoff_hz = float(sources[id].cutoff)
		player.attenuation_filter_db = float(sources[id].filter_db)
	player.set_meta("ssc_occlusion_gain",1.0)

func _advance_occlusion() -> void:
	if not realistic or not space:
		return
	for id in sources.keys():
		var player = sources[id].node.get_ref()
		if not is_instance_valid(player) or player.is_queued_for_deletion():
			sources.erase(id)
	var keys := sources.keys()
	if keys.is_empty():
		return
	var rays := 0
	var now := Time.get_ticks_msec()
	for step in keys.size():
		source_cursor %= keys.size()
		var state: Dictionary = sources[keys[source_cursor]]
		source_cursor = (source_cursor + 1) % keys.size()
		var player: AudioStreamPlayer3D = state.node.get_ref()
		if state.dry or not player.playing or int(state.next) > now or not player.is_inside_tree():
			continue
		state.next = now + 200
		var target := player.global_position
		if listener_position.distance_squared_to(target) < 0.01:
			_restore_source(player)
			continue
		var excluded: Array[RID] = []
		# Ignore a source's owning collision body, never arbitrary scene geometry.
		var ancestor: Node = player.get_parent()
		while ancestor:
			if ancestor is CollisionObject3D:
				excluded.append(ancestor.get_rid())
			ancestor = ancestor.get_parent()
		var owner_rid = player.get_meta("ssc_occlusion_exclude",null)
		if owner_rid is RID and owner_rid.is_valid():
			excluded.append(owner_rid)
		var occluded := obstructed(listener_position,target,excluded)
		player.set_meta("ssc_occlusion_gain",0.35 if occluded else 1.0)
		player.attenuation_filter_cutoff_hz = minf(float(state.cutoff),1200) if occluded else float(state.cutoff)
		player.attenuation_filter_db = minf(float(state.filter_db),-18) if occluded else float(state.filter_db)
		rays += 1
		if rays >= 4:
			break

func obstructed(origin: Vector3, target: Vector3, excluded: Array[RID] = []) -> bool:
	if not realistic or not space:
		return false
	var query := PhysicsRayQueryParameters3D.create(origin,target)
	query.exclude = excluded
	var hit := space.direct_space_state.intersect_ray(query)
	var position: Vector3 = hit.get("position",target)
	return not hit.is_empty() and position.distance_to(target) > 0.05

func noise_at(position: Vector3) -> float:
	for zone in zones:
		if zone.bounds.has_point(position):
			return clampf(float(zone.effects.get("noise",0)),0,effect_ceiling)
	return 0.0

func _update_noise() -> void:
	var amount := noise_at(listener_position)
	if amount <= 0 or not is_inside_tree():
		if noise_player:
			noise_player.stop()
		return
	if not noise_player:
		noise_player = AudioStreamPlayer.new()
		var generator := AudioStreamGenerator.new()
		generator.mix_rate = 48000
		generator.buffer_length = 0.1
		noise_player.stream = generator
		add_child(noise_player)
	noise_player.bus = route(listener_position)
	noise_player.volume_linear = 0.1 * amount
	if not noise_player.playing:
		noise_player.play()
	var playback := noise_player.get_stream_playback() as AudioStreamGeneratorPlayback
	if not playback:
		return
	# At most one 100-ms buffer, including low-frame-rate clients.
	var count := mini(playback.get_frames_available(),4800)
	var pcm := PackedVector2Array()
	pcm.resize(count)
	for index in count:
		var sample := noise_rng.randf_range(-1,1)
		pcm[index] = Vector2(sample,sample)
	playback.push_buffer(pcm)

func route(position: Vector3, dry: bool = false) -> StringName:
	if not dry:
		for zone in zones:
			if zone.bounds.has_point(position):
				return zone.bus
	return &"Master"

func clear() -> void:
	for state in sources.values():
		var player = state.node.get_ref()
		if is_instance_valid(player):
			_restore_source(player)
	sources.clear()
	source_cursor = 0
	space = null
	realistic = false
	if noise_player:
		noise_player.stop()
		noise_player.queue_free()
		noise_player = null
	dirty.clear()
	update_cursor = 0
	for name in bus_names:
		var index := AudioServer.get_bus_index(name)
		if index >= 0:
			AudioServer.remove_bus(index)
	bus_names.clear()
	zones.clear()

func _exit_tree() -> void:
	clear()
