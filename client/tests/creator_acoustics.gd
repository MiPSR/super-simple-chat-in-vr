extends RefCounted
const Acoustics = preload("res://scripts/acoustics.gd")

static func run() -> void:
	var acoustics := Acoustics.new()
	var before := AudioServer.bus_count
	acoustics.configure([{"id":"room","position":[0,0,0],"size":[10,10,10],"effects":{}}],0.25)
	var bus := AudioServer.get_bus_index(acoustics.zones[0].bus)
	assert(acoustics.creator_property("room","reverb",0.9).is_empty())
	assert(acoustics.creator_property("room","echo",0.5).is_empty())
	assert(acoustics.creator_property("room","compression",true).is_empty())
	assert(acoustics.dirty.size() == 1)
	acoustics._process(0)
	assert(acoustics.dirty.is_empty())
	assert(AudioServer.get_bus_effect_count(bus) == 4)
	var reverb := AudioServer.get_bus_effect(bus,0) as AudioEffectReverb
	assert(is_equal_approx(reverb.wet,0.25))
	assert(acoustics.creator_property("room","reverb",0.9).is_empty() and acoustics.dirty.is_empty())
	assert(not acoustics.creator_property("missing","reverb",0.5).is_empty())
	assert(not acoustics.creator_property("room","script","x").is_empty())
	assert(acoustics.route(Vector3.ZERO,true) == &"Master")
	acoustics.clear()
	assert(AudioServer.bus_count == before)
	acoustics.free()
	print("PASS creator acoustic zone updates, coalescing, administrative ceiling, dry path and cleanup")
