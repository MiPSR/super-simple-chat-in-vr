extends Control
## Circular stick menu, with a dead zone that makes release-to-cancel possible.
const ACTION_LABELS := {"microphone":"Mute / mic", "sleep":"Sleep", "phone_hand":"Phone hand", "interact":"Interact", "avatar_action":"Avatar action", "video_pause":"Video pause", "phone":"Phone controls", "leave":"Leave", "none":"Disabled"}
const DEFAULT_ACTIONS := ["microphone", "sleep", "phone_hand", "interact", "avatar_action", "video_pause", "phone", "leave"]
var actions: Array = DEFAULT_ACTIONS.duplicate()
var selected := -1

static func hold_delay(value: float) -> float:
	return clampf(value, 0.15, 1.5) if is_finite(value) else 0.45

func configure_actions(next: Array) -> bool:
	if next.size() != 8 or next.any(func(action): return not ACTION_LABELS.has(action)):
		return false
	actions = next.duplicate()
	selected = -1
	queue_redraw()
	return true

func selected_action() -> String:
	return str(actions[selected]) if selected >= 0 and selected < actions.size() else "none"

static func sector(stick: Vector2) -> int:
	if stick.length() < 0.45:
		return -1
	return posmod(int(round(atan2(stick.x, stick.y) / (TAU / 8.0))), 8)

func point(stick: Vector2) -> void:
	var next := sector(stick)
	if selected != next:
		selected = next
		queue_redraw()

func _draw() -> void:
	var center := size / 2.0
	var radius := minf(size.x, size.y) * 0.47
	draw_circle(center, radius, Color(0.03,0.05,0.09,0.97))
	var font := ThemeDB.fallback_font
	for index in 8:
		var angle := -PI / 2.0 + float(index) * TAU / 8.0
		var direction := Vector2.from_angle(angle)
		var edge := Vector2.from_angle(angle - PI / 8.0)
		draw_line(center + edge * 45, center + edge * radius, Color(0.3,0.4,0.5), 2)
		if index == selected:
			draw_circle(center + direction * radius * 0.7, radius * 0.23, Color(0.15,0.45,0.65))
		var text: String = ACTION_LABELS[actions[index]]
		var extent := font.get_string_size(text, HORIZONTAL_ALIGNMENT_LEFT, -1, 18)
		draw_string(font, center + direction * radius * 0.7 - extent / 2.0 + Vector2(0,14), text, HORIZONTAL_ALIGNMENT_LEFT, -1, 18)
	draw_circle(center, 43, Color(0.08,0.1,0.15))
	draw_string(font, center + Vector2(-30,6), "Cancel", HORIZONTAL_ALIGNMENT_LEFT, -1, 18)
