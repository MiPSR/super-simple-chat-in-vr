extends SceneTree
var started := 0

func _initialize() -> void:
	started = Time.get_ticks_msec()
	call_deferred("_run")

func _run() -> void:
	var app: Node = load("res://main.tscn").instantiate()
	root.add_child(app)
	app.loader.base_url = OS.get_environment("SSC_TEST_API")
	app.loader.credentials = OS.get_environment("SSC_TEST_TOKEN")
	var state := {"saved":false}
	app.preferences_request.request_completed.connect(func(_result: int, code: int, _headers: PackedStringArray, body: PackedByteArray):
		var response = JSON.parse_string(body.get_string_from_utf8())
		if code == 200 and response is Dictionary and response.get("ok", false):
			state.saved = true
	)
	var headers := PackedStringArray(["Authorization: Bearer " + app.loader.credentials])
	assert(app.preferences_request.request(app.loader.base_url + "/api/v1/me", headers) == OK)
	while app.preferences.is_empty():
		await process_frame
	app.preferences.voice_volume = 0.3
	app.preferences.send_divisor = 4
	app.preferences.texture_reduction = 2
	app.preferences.draw_distance = 80.0
	app.preferences.radial_hold_seconds = 0.7
	app.preferences.radial_actions = ["phone","none","phone_hand","interact","avatar_action","video_pause","sleep","leave"]
	app._save_preferences()
	app.preferences.voice_volume = 0.4
	app._save_preferences() # Coalesce rapid edits; persist only the newest state.
	while not state.saved:
		await process_frame
	await process_frame
	app.preferences = {}
	assert(app.preferences_request.request(app.loader.base_url + "/api/v1/me", headers) == OK)
	while app.preferences.is_empty():
		await process_frame
	assert(is_equal_approx(float(app.preferences.voice_volume), 0.4))
	assert(int(app.preferences.send_divisor) == 4)
	assert(int(app.preferences.texture_reduction) == 2 and app.loader.texture_reduction == 2)
	assert(is_equal_approx(float(app.preferences.draw_distance),80.0) and is_equal_approx(app.camera.far,80.0))
	assert(is_equal_approx(float(app.preferences.radial_hold_seconds), 0.7))
	assert(app.preferences.radial_actions[0] == "phone" and app.preferences.radial_actions[1] == "none")
	assert(is_equal_approx(root.scaling_3d_scale, 0.75))
	assert(root.msaa_3d == Viewport.MSAA_4X)
	assert(not app.voice.microphone_enabled)
	app._controller_pressed(&"menu_button")
	assert(is_equal_approx(app.menu_hold_delay, 0.7))
	app.preferences.radial_hold_seconds = 0.15
	app.preferences.radial_actions[0] = "leave"
	app.radial.selected = 0
	assert(app.radial.selected_action() == "phone", "An active gesture must keep its captured bindings")
	app.menu_held = 0.71
	app._controller_released(&"menu_button")
	assert(app.phone.visible and not app.voice.microphone_enabled)
	app.free()
	await process_frame
	print("PASS real client account-preference load, coalesced HTTP save, integer normalization, reload and microphone privacy")
	quit()

func _process(_delta: float) -> bool:
	if Time.get_ticks_msec() - started > 10000:
		push_error("Client preference HTTP test timed out")
		quit(2)
	return false
