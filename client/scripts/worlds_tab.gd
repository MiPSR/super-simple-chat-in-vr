extends VBoxContainer
## Worlds tab: catalog/instance browsing; joining mints a one-use grant via
## the asset API, then the caller feeds it through the paste-grant flow.
signal status(text: String)
signal join_requested(grant: Dictionary)
const GRANT_FIELDS := ["ticket", "secret", "port", "api_url", "api_token"]
var request: HTTPRequest
var base_url := ""
var credentials := ""
var send_divisor := 1
var receive_divisor := 1
var world_column: VBoxContainer
var instance_column: VBoxContainer
var pending := ""

func _ready() -> void:
	request = HTTPRequest.new()
	request.body_size_limit = 2_000_000
	add_child(request)
	request.request_completed.connect(_completed)
	var title := Label.new()
	title.text = "Worlds and instances available to your grant"
	add_child(title)
	var refresh := Button.new()
	refresh.text = "Refresh lists"
	refresh.custom_minimum_size.y = 54
	refresh.pressed.connect(func(): _refresh())
	add_child(refresh)
	world_column = _section("Worlds")
	instance_column = _section("Instances")

func _section(title: String) -> VBoxContainer:
	var label := Label.new()
	label.text = title
	label.add_theme_font_size_override("font_size", 24)
	add_child(label)
	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 10)
	add_child(column)
	return column

func configure(url: String, token: String, send: int, receive: int) -> void:
	base_url = url
	credentials = token
	send_divisor = send if [1, 2, 4].has(send) else 1
	receive_divisor = receive if [1, 2, 4].has(receive) else 1
	_refresh()

func _refresh() -> void:
	_load("/api/v1/assets?kind=world", "worlds")

func _load(path: String, mode: String) -> void:
	if base_url.is_empty() or credentials.is_empty():
		return
	if request.get_http_client_status() != HTTPClient.STATUS_DISCONNECTED:
		status.emit("Still waiting on the asset API; try again shortly")
		return
	pending = mode
	var error := request.request(base_url + path, PackedStringArray(["Authorization: Bearer " + credentials]))
	if error != OK:
		pending = ""
		status.emit("Could not reach the asset API")

func _join(kind: String, id: String) -> void:
	if base_url.is_empty() or credentials.is_empty():
		return
	if request.get_http_client_status() != HTTPClient.STATUS_DISCONNECTED:
		status.emit("Still waiting on the asset API; try again shortly")
		return
	pending = "join"
	var error := request.request(base_url + "/api/v1/game-grants", PackedStringArray(["Authorization: Bearer " + credentials,"Content-Type: application/json"]), HTTPClient.METHOD_POST, JSON.stringify({kind: id, "send_divisor": send_divisor, "receive_divisor": receive_divisor}))
	if error != OK:
		pending = ""
		status.emit("Could not request a join grant")

func _completed(result: int, code: int, _headers: PackedStringArray, body: PackedByteArray) -> void:
	var mode := pending
	pending = ""
	if result != HTTPRequest.RESULT_SUCCESS:
		status.emit("Asset API request failed (%s)" % result)
		return
	if code != 200:
		status.emit("Request failed (HTTP %s): %s" % [code, _http_note(code)])
		return
	var data = JSON.parse_string(body.get_string_from_utf8())
	match mode:
		"worlds":
			_render_worlds(data if data is Array else [])
			_load("/api/v1/instances", "instances")
		"instances":
			_render_instances(data if data is Array else [])
		"join":
			if data is Dictionary:
				_grant(data)
			else:
				status.emit("Malformed join grant response")

static func _http_note(code: int) -> String:
	match code:
		401: return "sign in again or paste a fresh grant"
		404: return "world or instance is gone"
		429: return "rate limited; wait before retrying"
		_: return "unexpected response"

func _grant(data: Dictionary) -> void:
	if not data.has_all(GRANT_FIELDS):
		status.emit("Incomplete join grant response")
		return
	data.host = _host_from_url(str(data.api_url))
	join_requested.emit(data)

static func _host_from_url(url: String) -> String:
	# The web grant flow ships the hostname alongside the game grant; the API
	# response carries it only inside api_url.
	var text := url.trim_prefix("https://").trim_prefix("http://")
	var cut := text.find("/")
	if cut >= 0:
		text = text.substr(0, cut)
	cut = text.rfind(":")
	if cut >= 0:
		text = text.substr(0, cut)
	return text

func _render_worlds(list: Array) -> void:
	for child in world_column.get_children():
		child.queue_free()
	for row in list:
		var card := VBoxContainer.new()
		var name := Label.new()
		name.text = str(row.get("name", row.get("id", "")))
		card.add_child(name)
		var description := Label.new()
		description.text = str(row.get("description", ""))
		description.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
		description.custom_minimum_size = Vector2(500, 0)
		card.add_child(description)
		card.add_child(_join_button("Join world", "world", str(row.get("id", ""))))
		world_column.add_child(card)

func _render_instances(list: Array) -> void:
	for child in instance_column.get_children():
		child.queue_free()
	for row in list:
		var card := VBoxContainer.new()
		var title := Label.new()
		title.text = "%s · %s/%s%s" % [str(row.get("world", "")), str(row.get("players", 0)), str(row.get("capacity", 0)), " · private" if bool(row.get("private", false)) else ""]
		card.add_child(title)
		var names := PackedStringArray()
		for player in row.get("roster", []):
			names.append(str(player.get("name", "")))
		var roster := Label.new()
		roster.text = ", ".join(names)
		card.add_child(roster)
		card.add_child(_join_button("Join instance", "instance", str(row.get("id", ""))))
		instance_column.add_child(card)

func _join_button(label: String, kind: String, id: String) -> Button:
	var button := Button.new()
	button.text = label
	button.custom_minimum_size.y = 54
	button.pressed.connect(_join.bind(kind, id))
	return button
