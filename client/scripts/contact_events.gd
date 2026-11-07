extends RefCounted
## Only compare authenticated, viewer-filtered server snapshots. No permissions
## or shared outcomes are inferred from local collision/hand positions.
var previous: Dictionary = {}
var explicit_releases: Dictionary = {}

static func same_contact(a: Dictionary, b: Dictionary) -> bool:
	return a.get("target") == b.get("target") and a.get("segment") == b.get("segment")

func acknowledge(event: Dictionary) -> void:
	var key := str(event.get("actor", "")) + ":" + str(int(event.get("hand", 0)))
	if event.get("action") == "release" and previous.has(key):
		var old: Dictionary = previous[key]
		if event.get("target") == old.get("target") and (not event.has("segment") or event.segment == old.get("segment")):
			explicit_releases[key] = old.duplicate(true)
	elif event.get("action") == "grab":
		explicit_releases.erase(key)

func replace(next: Dictionary) -> Array[Dictionary]:
	var releases: Array[Dictionary] = []
	for key in previous:
		var old: Dictionary = previous[key]
		if not next.has(key) or not same_contact(old, next[key]):
			if not explicit_releases.has(key) or not same_contact(old, explicit_releases[key]):
				var event := old.duplicate(true)
				event.action = "release"
				event.kind = "avatar"
				event.reason = "snapshot"
				releases.append(event)
			explicit_releases.erase(key)
	previous = next.duplicate(true)
	return releases

func clear() -> void:
	previous.clear()
	explicit_releases.clear()
