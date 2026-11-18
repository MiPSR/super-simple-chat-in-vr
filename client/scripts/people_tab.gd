extends VBoxContainer
## People tab: per-player block flags. Server enforcement follows within ~5s.
signal status(text: String)
const KEYS := ["voice", "text", "touch", "avatar", "complete"]
var request: HTTPRequest
var base_url := ""
var credentials := ""
var rows: Dictionary = {}
var blocks: Dictionary = {}
var puts: Dictionary = {}
var populating := false

static func pack_flags(voice: bool, text: bool, touch: bool, avatar: bool, complete: bool) -> int:
	return (1 if voice else 0) | (2 if text else 0) | (4 if touch else 0) | (8 if avatar else 0) | (16 if complete else 0)

static func unpack_flags(flags: int) -> Dictionary:
	return {"voice": flags & 1 != 0, "text": flags & 2 != 0, "touch": flags & 4 != 0, "avatar": flags & 8 != 0, "complete": flags & 16 != 0}

func _ready() -> void:
	request = HTTPRequest.new()
	request.body_size_limit = 100_000
	add_child(request)
	request.request_completed.connect(_completed)
	var note := Label.new()
	note.text = "applies within ~5s"
	note.add_theme_font_size_override("font_size", 20)
	add_child(note)

func join(url: String, token: String) -> void:
	base_url = url
	credentials = token
	var error := request.request(base_url + "/api/v1/blocks", PackedStringArray(["Authorization: Bearer " + credentials]))
	if error != OK:
		status.emit("Could not request your block list")

func clear() -> void:
	for id in rows.keys():
		rows[id].node.queue_free()
	rows.clear()
	blocks.clear()
	puts.clear()
	base_url = ""
	credentials = ""

func players(sample: Array, self_id: String) -> void:
	var seen: Dictionary = {}
	for player in sample:
		var id := str(player.get("id", ""))
		if id.is_empty() or id == self_id:
			continue
		seen[id] = true
		if not rows.has(id):
			_build_row(id, str(player.get("name", id)))
			blocks[id] = int(blocks.get(id, 0))
			_set_row(id, int(blocks[id]))
	for id in rows.keys():
		if not seen.has(id):
			rows[id].node.queue_free()
			rows.erase(id)

func _build_row(id: String, name: String) -> VBoxContainer:
	var row := VBoxContainer.new()
	var label := Label.new()
	label.text = name
	row.add_child(label)
	var flags := HBoxContainer.new()
	flags.add_theme_constant_override("separation", 18)
	row.add_child(flags)
	var checks: Dictionary = {}
	for key in KEYS:
		var check := CheckButton.new()
		check.text = key.capitalize()
		check.toggled.connect(_flag_toggled.bind(id))
		flags.add_child(check)
		checks[key] = check
	rows[id] = {"node":row, "checks":checks}
	add_child(row)
	return row

func _flag_toggled(_pressed: bool, id: String) -> void:
	if populating or not rows.has(id):
		return
	var row: Dictionary = rows[id]
	var next := pack_flags(row.checks.voice.button_pressed, row.checks.text.button_pressed, row.checks.touch.button_pressed, row.checks.avatar.button_pressed, row.checks.complete.button_pressed)
	blocks[id] = next
	puts[id] = next
	_flush()

func _set_row(id: String, flags: int) -> void:
	if not rows.has(id):
		return
	var row: Dictionary = rows[id]
	var state: Dictionary = unpack_flags(flags)
	for key in KEYS:
		row.checks[key].set_pressed_no_signal(bool(state[key]))

func _flush() -> void:
	if puts.is_empty() or base_url.is_empty():
		return
	if request.get_http_client_status() != HTTPClient.STATUS_DISCONNECTED:
		return # The completion callback retries the queued flags.
	var target := str(puts.keys()[0])
	var flags := int(puts[target])
	puts.erase(target)
	var error := request.request(base_url + "/api/v1/blocks/" + target, PackedStringArray(["Authorization: Bearer " + credentials,"Content-Type: application/json"]), HTTPClient.METHOD_PUT, JSON.stringify({"flags":flags}))
	if error != OK:
		puts[target] = flags
		status.emit("Could not save block flags")

func _completed(result: int, code: int, _headers: PackedStringArray, body: PackedByteArray) -> void:
	_flush()
	if result != HTTPRequest.RESULT_SUCCESS:
		status.emit("Block list request failed (%s)" % result)
		return
	if code != 200:
		status.emit("Block list request failed (HTTP %s)" % code)
		return
	var data = JSON.parse_string(body.get_string_from_utf8())
	if not data is Dictionary or not data.get("blocks") is Array:
		return
	var found: Dictionary = {}
	for row in data.blocks:
		found[str(row.get("target", ""))] = int(row.get("flags", 0))
	populating = true
	for id in rows.keys():
		_set_row(id, int(found.get(id, 0)))
		blocks[id] = int(found.get(id, 0))
	populating = false
