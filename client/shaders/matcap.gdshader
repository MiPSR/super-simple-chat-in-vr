shader_type spatial;
render_mode unshaded, blend_add, depth_draw_never, cull_back, shadows_disabled;

uniform sampler2D matcap_texture : source_color, filter_linear_mipmap, repeat_disable;
uniform sampler2D albedo_texture : source_color, hint_default_white;
uniform float strength : hint_range(0.0, 1.0) = 0.25;
uniform float base_alpha : hint_range(0.0, 1.0) = 1.0;

void fragment() {
    vec3 view_normal = NORMAL / max(length(NORMAL), 0.0001);
    vec2 matcap_uv = view_normal.xy * vec2(0.5, -0.5) + vec2(0.5);
    ALBEDO = texture(matcap_texture, matcap_uv).rgb * strength;
    ALPHA = texture(albedo_texture, UV).a * base_alpha;
}
