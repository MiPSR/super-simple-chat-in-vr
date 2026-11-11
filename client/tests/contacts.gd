extends RefCounted
const Events = preload("res://scripts/contact_events.gd")
const Creator = preload("res://scripts/creator_runtime.gd")
const Avatar = preload("res://scripts/avatar.gd")
class Scope extends Node3D:
	var received: Array = []
	func creator_action(action: String, target: String, weight: float) -> void:
		received.append({"action":action,"target":target,"weight":weight})

static func run() -> void:
	var events := Events.new()
	var grab := {"actor":"a","target":"b","segment":"hair","bone":"Hair","hand":0}
	assert(events.replace({"a:0":grab}).is_empty())
	var releases := events.replace({})
	assert(releases.size() == 1 and releases[0].reason == "snapshot")
	assert(releases[0].actor == "a" and releases[0].target == "b" and releases[0].segment == "hair")
	var scope := Scope.new()
	Engine.get_main_loop().root.add_child(scope)
	var creator := Creator.new()
	scope.add_child(creator)
	creator.set_process(false) # Exercise the synchronous tooling path explicitly.
	creator.scope = scope
	var vm = ClassDB.instantiate("CreatorVm")
	assert(vm.load_script("avatar.on('release', function(e) assert(e.reason == 'snapshot' and e.target == 'b'); avatar.expression('released', 1) end)", PackedStringArray(["visual"])) == "")
	creator.scripts.append({"vm":vm,"attachment":"avatar","failed":false})
	var avatar := Avatar.new()
	avatar.creator = creator
	avatar.contact(releases[0])
	assert(scope.received.size() == 1 and scope.received[0].target == "released")
	avatar.free()
	scope.free()
	assert(events.replace({}).is_empty())
	events.replace({"a:0":grab})
	var explicit := grab.duplicate()
	explicit.action = "release"
	events.acknowledge(explicit)
	assert(events.replace({"a:0":grab}).is_empty()) # Old snapshot before removal.
	assert(events.replace({}).is_empty()) # Explicit callback already delivered.
	assert(events.explicit_releases.is_empty())
	events.replace({"a:0":grab})
	var replacement := grab.duplicate()
	replacement.target = "c"
	assert(events.replace({"a:0":replacement}).size() == 1)
	assert(events.replace({}).size() == 1)
	events.replace({"a:0":grab})
	events.acknowledge(explicit)
	var new_grab := grab.duplicate()
	new_grab.action = "grab"
	events.acknowledge(new_grab)
	assert(events.replace({}).size() == 1, "A new grab must retire the old explicit-release marker")
	events.replace({"a:0":grab})
	events.clear()
	assert(events.replace({}).is_empty() and events.explicit_releases.is_empty())
	print("PASS snapshot-driven contact releases, explicit-event deduplication, replacement and teardown")
