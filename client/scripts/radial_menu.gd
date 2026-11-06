extends Control
## Circular stick menu, with a dead zone that makes release-to-cancel possible.
const LABELS := ["Mute / mic", "Sleep", "Phone hand", "Interact", "Avatar action", "Video pause", "Phone controls", "Leave"]
var selected := -1

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
		var text: String = LABELS[index]
		var extent := font.get_string_size(text, HORIZONTAL_ALIGNMENT_LEFT, -1, 18)
		draw_string(font, center + direction * radius * 0.7 - extent / 2.0 + Vector2(0,14), text, HORIZONTAL_ALIGNMENT_LEFT, -1, 18)
	draw_circle(center, 43, Color(0.08,0.1,0.15))
	draw_string(font, center + Vector2(-30,6), "Cancel", HORIZONTAL_ALIGNMENT_LEFT, -1, 18)
