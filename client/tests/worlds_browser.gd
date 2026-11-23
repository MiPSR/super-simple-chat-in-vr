extends RefCounted

class Browser extends "res://scripts/worlds_tab.gd":
	var sent: Array = []
	func _post(path: String, mode: String, payload: Dictionary) -> void:
		sent.append({"path": path, "mode": mode, "payload": payload})
		pending = mode
	func _load(path: String, mode: String) -> void:
		sent.append({"path": path, "mode": mode})

static func run() -> void:
	var tree: SceneTree = Engine.get_main_loop()
	var browser := Browser.new()
	tree.root.add_child(browser)
	var notices: Array = []
	var grants: Array = []
	browser.status.connect(func(text): notices.append(text))
	browser.join_requested.connect(func(grant): grants.append(grant))
	browser.search_input.text = " a & b "
	browser._refresh()
	assert(browser.sent[-1].path == "/api/v1/assets?kind=world&q=a%20%26%20b")
	browser.send_divisor = 2
	browser.receive_divisor = 4
	browser._render_worlds([{"id":"world1","name":"World"}])
	var world: Node = browser.world_column.get_child(0)
	world.get_child(3).pressed.emit()
	assert(browser.sent[-1].payload == {"world":"world1","private":true,"send_divisor":2,"receive_divisor":4})
	browser._render_instances([
		{"id":"mine","can_invite":true},
		{"id":"other","can_invite":false},
		{"id":"mine2","can_invite":true},
	])
	var mine: Node = browser.instance_column.get_child(0)
	assert(browser.instance_column.get_child(1).get_child_count() == 3)
	mine.get_child(3).text = "123456789"
	mine.get_child(4).pressed.emit()
	assert(browser.sent[-1].path == "/api/v1/instances/mine/invite")
	assert(browser.sent[-1].payload == {"user":"123456789"})
	browser._completed(HTTPRequest.RESULT_SUCCESS, 200, PackedStringArray(), '{"invitation":"token1","expires_in":300}'.to_utf8_buffer())
	assert(JSON.parse_string(browser.invitation_output.text) == {"instance":"mine","invitation":"token1"})
	var second: Node = browser.instance_column.get_child(2)
	second.get_child(3).text = "987654321"
	second.get_child(4).pressed.emit()
	assert(browser.sent[-1].path == "/api/v1/instances/mine2/invite", "Each card must retain its own instance")
	browser._completed(HTTPRequest.RESULT_SUCCESS, 200, PackedStringArray(), '{"invitation":"token2"}'.to_utf8_buffer())
	var count := browser.sent.size()
	browser._invite("mine", "１２３４５")
	browser._invite("mine", "-12345")
	assert(browser.sent.size() == count, "Invalid invitee IDs must not be sent")
	browser.invite_instance.text = " mine "
	browser.invite_token.text = " token1 "
	browser._redeem_invitation()
	assert(browser.sent[-1].payload == {"instance":"mine","invitation":"token1","send_divisor":2,"receive_divisor":4})
	browser._completed(HTTPRequest.RESULT_SUCCESS, 403, PackedStringArray(), "{}".to_utf8_buffer())
	assert(grants.is_empty() and browser.invite_token.text == " token1 ")
	assert("not permitted" in notices[-1])
	# Failed responses never switch the active game session.
	browser.pending = "join"
	browser._completed(HTTPRequest.RESULT_SUCCESS, 200, PackedStringArray(), "{}".to_utf8_buffer())
	assert(grants.is_empty())
	browser.pending = "join"
	var grant := {"ticket":"ticket","secret":"secret","port":1234,"api_url":"https://example.com:443","api_token":"token"}
	browser._completed(HTTPRequest.RESULT_SUCCESS, 200, PackedStringArray(), JSON.stringify(grant).to_utf8_buffer())
	assert(grants.size() == 1 and grants[0].host == "example.com")
	assert(browser.invite_token.text.is_empty())
	browser.queue_free()
	await tree.process_frame
	print("PASS in-game world search, private creation, owner invitation controls, redemption, errors and rate preferences")
