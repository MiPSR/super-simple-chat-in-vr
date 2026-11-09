extends RefCounted
const Main = preload("res://scripts/main.gd")

class CreatorSpy extends Node:
	var updates: Array[Dictionary] = []
	var events: Array = []
	func update_objects(objects: Dictionary, tick: int) -> void:
		updates.append({"objects":objects.duplicate(true),"tick":tick})
	func queue_world_events(values: Array) -> void:
		events.append_array(values.duplicate(true))

static func run() -> void:
	# No _ready/UI/XR is needed to exercise the actual main packet handler.
	var main := Main.new()
	var spy := CreatorSpy.new()
	main.add_child(spy)
	main.creator = spy
	var prop := Node3D.new()
	main.add_child(prop)
	main.latest_tick = 10
	main.entity_nodes = {"prop":prop}
	main.entity_data = {"prop":{"id":"prop","position":[3,0,0]}}
	var events := [{"event":"spawn","entity":{"id":"prop","position":[0,0,0]}},{"event":"destroy","target":"prop"}]
	for tick in [9,10]:
		main._message(11,{"tick":tick,"events":events})
		assert(main.entity_data.prop.position == [3,0,0])
		assert(main.entity_nodes.prop == prop and not prop.is_queued_for_deletion())
	assert(spy.updates.is_empty() and spy.events.size() == 4)
	main._message(11,{"tick":11,"events":[{"event":"destroy","target":"prop"}]})
	assert(main.entity_data.is_empty() and main.entity_nodes.is_empty())
	assert(prop.is_queued_for_deletion())
	assert(spy.updates.size() == 1 and spy.updates[0].objects.is_empty() and spy.updates[0].tick == 11)
	main.free()
	print("PASS actual client notifications preserve newer/same-tick graphs, avoid pose regression and retain callbacks")
