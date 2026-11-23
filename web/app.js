"use strict";
let adminSecret = "";
let me;
let operatorSettings;
const policyFields = [
  ["tps","Simulation TPS",1,120],["instance_limit","Maximum instances",1,512],["instance_players","Players per instance",1,128],
  ["script_policy.memory_bytes","Creator VM memory bytes",1048576,8388608],
  ["script_policy.callback_millis","Creator callback CPU budget (ms)",1,5],
  ["script_policy.scheduler_millis","Creator scheduler budget per tick (ms)",1,8],
  ["network_rates.minimum_hz","Minimum network Hz",1,120],["network_rates.maximum_hz","Maximum network Hz",1,120],
  ["network_rates.automatic","Adapt rates for poor connections","boolean"],["network_rates.suppress_unchanged","Suppress unchanged pose updates","boolean"],
  ["default_world_slots","New-account world slots",1,1000],["default_avatar_slots","New-account avatar slots",1,1000],
  ["default_world_bytes","Default world package bytes",0,1000000000],["default_avatar_bytes","Default avatar package bytes",0,1000000000],
  ["voice_bitrate","Voice bitrate",8000,64000],["audio_effect_max","Audio effects ceiling",0,1,0.05],
  ["radio_bitrate","Radio bitrate",8000,16000],["radio_channels","Radio channels (comma-separated; empty disables channels)","channels"],
  ["radio_presets","Radio presets (standard, narrow, weak)","list"],
  ["radio_noise","Radio noise",0,0.8,0.05],["radio_interference","Radio interference",0,0.8,0.05],
  ["radio_filter_hz","Radio filter Hz",300,3000],["radio_ft8_interval","FT8-style interval (seconds)",1,120],["radio_sstv_interval","SSTV interval (seconds)",5,600]
];
const policyValue = (settings,path) => path.split(".").reduce((value,key) => value?.[key],settings);
function renderPolicies(data) {
  operatorSettings = data.saved;
  const fields = $("policy-fields"); fields.replaceChildren();
  for (const [path,title,min,max,step] of policyFields) {
    const value = policyValue(operatorSettings,path);
    if (value === undefined) continue;
    const label = document.createElement("label"); label.textContent = title;
    const input = document.createElement("input"); input.name = path;
    input.type = min === "boolean" ? "checkbox" : typeof min === "number" ? "number" : "text";
    if (input.type === "checkbox") input.checked = value;
    else if (input.type === "number") { input.min = min; input.max = max; input.step = step || 1; input.required = true; input.value = value; }
    else { input.value = value.join(","); input.maxLength = 256; }
    label.append(input); fields.append(label);
  }
  $("policy-status").textContent = data.restart_required ? "Saved changes are pending a server restart." : "Saved policies match the running server.";
}
const $ = id => document.getElementById(id);
const status = message => { $("status").textContent = message; };
const radialActions = {microphone:"Mute / microphone",sleep:"Sleep",phone_hand:"Phone hand",interact:"Interact",avatar_action:"Avatar action",video_pause:"Video pause",phone:"Phone controls",leave:"Leave",none:"Disabled"};
for (let index = 0; index < 8; index++) {
  const label = document.createElement("label");
  label.textContent = `Quick menu sector ${index + 1} `;
  const select = document.createElement("select");
  select.name = `radial_action_${index}`;
  for (const [value, text] of Object.entries(radialActions)) {
    const option = document.createElement("option"); option.value = value; option.textContent = text; select.append(option);
  }
  label.append(select); $("radial-actions").append(label);
}
async function api(path, options = {}) {
  const headers = { ...options.headers };
  if (adminSecret) headers.Authorization = `Bearer ${adminSecret}`;
  if (options.body && !(options.body instanceof FormData)) {
    headers["Content-Type"] = "application/json";
    options.body = JSON.stringify(options.body);
  }
  const response = await fetch(`/api/v1${path}`, { ...options, headers, credentials: "same-origin" });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || response.statusText);
  return result;
}
function action(form, fn) {
  $(form).addEventListener("submit", async event => {
    event.preventDefault();
    const button = event.submitter;
    if (button) button.disabled = true;
    try { await fn(event); } catch (error) { status(error.message); }
    finally { if (button) button.disabled = false; }
  });
}
function button(label, fn) {
  const b = document.createElement("button"); b.textContent = label;
  b.onclick = async () => { b.disabled = true; try { await fn(); } catch (e) { status(e.message); } finally { b.disabled = false; } };
  return b;
}
function article(title, text) {
  const a = document.createElement("article"), h = document.createElement("h3"), p = document.createElement("p");
  h.textContent = title; p.textContent = text; a.append(h,p); return a;
}
async function grant(join) {
  const data = await api("/game-grants", { method: "POST", body: { ...join, send_divisor: me.preferences.send_divisor, receive_divisor: me.preferences.receive_divisor } });
  data.host = location.hostname;
  $("grant").value = JSON.stringify(data); $("grant").select();
  status("One-use grant created. Paste it into the client within 60 seconds.");
}
const quotaFields = [["world_slots", "World slots", 1000], ["avatar_slots", "Avatar slots", 1000], ["world_bytes", "World bytes", 1000000000], ["avatar_bytes", "Avatar bytes", 1000000000]];
function quotaForm(u) {
  const form = document.createElement("form");
  for (const [name, label, max] of quotaFields) {
    const l = document.createElement("label");
    l.textContent = `${label} `;
    const input = document.createElement("input");
    input.name = name; input.type = "number"; input.min = 0; input.max = max; input.value = u[name];
    l.append(input); form.append(l);
  }
  const save = document.createElement("button");
  save.textContent = "Save quotas";
  form.append(save);
  form.addEventListener("submit", async event => {
    event.preventDefault();
    const submitter = event.submitter;
    if (submitter) submitter.disabled = true;
    try {
      const body = {};
      for (const [name] of quotaFields) body[name] = Number(form.elements.namedItem(name).value);
      await api(`/admin/users/${u.id}`, { method: "PUT", body });
      status("Quotas saved.");
      await refresh();
    } catch (error) { status(error.message); }
    finally { if (submitter) submitter.disabled = false; }
  });
  return form;
}
async function refresh() {
  const assets = await api(`/assets?q=${encodeURIComponent($("query").value)}`);
  $("assets").replaceChildren(...assets.map(asset => {
    const card = article(`${asset.name}${asset.enabled ? "" : " (disabled)"}`, `${asset.kind} · ${asset.id}\n${asset.description}`);
    if (asset.kind === "world") {
      card.append(button("New public instance", () => grant({ world: asset.id })), button("New invite-only instance", () => grant({ world: asset.id, private: true })));
    }
    if (me.user.admin) card.append(button(asset.enabled ? "Disable listing" : "Enable listing", async () => {
      try { await api(`/admin/assets/${asset.id}/enabled`, { method: "POST", body: { enabled: !asset.enabled } }); }
      catch (error) { status(error.message); } // A 404 means the asset is gone; the refresh below drops it.
      await refresh();
    }));
    if (asset.owner === me.user.id || me.user.admin) card.append(button("Delete", async () => { if (confirm(`Delete ${asset.name}? Running instances keep their version.`)) { await api(`/assets/${asset.id}`, { method: "DELETE" }); await refresh(); } }));
    return card;
  }));
  const instances = await api("/instances");
  $("instances").replaceChildren(...instances.map(i => {
    const roster = i.roster || [];
    const names = roster.slice(0, 12).map(p => p.name || p.id);
    const extra = roster.length - names.length;
    const a = article(i.id, `${i.players}/${i.capacity} players · ${i.private ? "Invite-only" : "Public"}${names.length ? ` · ${names.join(", ")}${extra > 0 ? ` +${extra} more` : ""}` : ""}`);
    a.append(button("Join", () => grant({ instance: i.id })));
    if (i.can_invite) a.append(button("Invite user", async () => {
      const user = prompt("Discord ID to invite"); if (!user) return;
      const invite = await api(`/instances/${i.id}/invite`, { method: "POST", body: { user } });
      status(`Invite: ${JSON.stringify({instance: i.id, ...invite})}`);
    })); return a;
  }));
  if (me.user.admin) {
    const users = await api("/admin/users");
    $("users").replaceChildren(...users.map(u => {
      const a = article(`${u.name || u.id} ${u.enabled ? "" : "(disabled)"}`, `${u.id}: ${u.world_slots} worlds / ${u.avatar_slots} avatars`);
      if (u.id !== "admin") a.append(button(u.enabled ? "Disable & revoke sessions" : "Re-enable account", async () => {
        await api(`/admin/users/${u.id}/enabled`, { method: "POST", body: { enabled: !u.enabled } });
        await refresh();
      }));
      a.append(quotaForm(u)); return a;
    }));
    const health = await api("/admin/health");
    if (health.limits) $("limits").textContent = `Live-instance limits: ${health.limits.instances} instances · ${health.limits.instance_players} players per instance.`;
    if (health.defaults) $("defaults").textContent = `New-account quotas: ${health.defaults.world_slots} world slots / ${health.defaults.world_bytes} bytes · ${health.defaults.avatar_slots} avatar slots / ${health.defaults.avatar_bytes} bytes.`;
    $("health").textContent = JSON.stringify(health, null, 2);
    const logs = await api("/admin/logs");
    $("logs").textContent = logs.map(entry => `${new Date(entry.time * 1000).toISOString()} ${entry.line}`).join("\n") || "No recent entries.";
    renderPolicies(await api("/admin/settings"));
  }
}
async function load() {
  me = await api("/me");
  $("account").hidden = false; $("admin").hidden = !me.user.admin; $("server-policy").hidden = !me.user.admin;
  $("identity").textContent = `Signed in as ${me.user.id}`;
  for (const [key,value] of Object.entries(me.preferences)) {
    if (key === "radial_actions") { value.forEach((action, index) => { $("preferences").elements.namedItem(`radial_action_${index}`).value = action; }); continue; }
    const input = $("preferences").elements.namedItem(key);
    if (input) { if (input.type === "checkbox") input.checked = value; else input.value = value; }
  }
  await refresh(); status("Ready.");
}
action("admin-login", async () => { adminSecret = $("admin-secret").value; $("admin-secret").value = ""; try { await load(); } catch(e) {adminSecret = ""; throw e;} });
action("search", refresh);
action("server-settings", async () => {
  if (!operatorSettings) throw new Error("Load administrator settings first.");
  const saved = JSON.parse(JSON.stringify(operatorSettings));
  for (const [path,,kind] of policyFields) {
    const input = $("server-settings").elements.namedItem(path);
    if (!input) continue;
    const value = kind === "boolean" ? input.checked : kind === "channels" ? input.value.split(",").map(v => v.trim()).filter(Boolean).map(Number) : kind === "list" ? input.value.split(",").map(v => v.trim()) : Number(input.value);
    const keys = path.split("."); const key = keys.pop();
    keys.reduce((value,key) => value[key],saved)[key] = value;
  }
  const result = await api("/admin/settings", {method:"PUT",body:saved});
  operatorSettings = saved;
  $("policy-status").textContent = result.restart_required ? "Policies saved. Restart the server service to apply them." : "Policies saved; no restart needed.";
  status("Server policies saved.");
});
action("join-invitation", async () => {
  await grant({instance:$("invite-instance").value.trim(),invitation:$("invite-token").value.trim()});
  $("invite-token").value = "";
});
action("preferences", async () => {
  const p = { ...me.preferences };
    for (const key of Object.keys(p)) { const el = $("preferences").elements.namedItem(key); if (!el) continue; p[key] = el.type === "checkbox" ? el.checked : ["quality", "send_divisor", "receive_divisor", "voice_volume", "microphone_gain", "resolution_scale", "draw_distance", "texture_lod_bias", "texture_reduction", "radial_hold_seconds", "turn_angle", "move_speed"].includes(key) ? Number(el.value) : el.value; }
  p.radial_actions = Array.from({length:8}, (_, index) => $("preferences").elements.namedItem(`radial_action_${index}`).value);
  await api("/preferences", {method:"PUT",body:p}); me.preferences = p; status("Preferences saved.");
});
action("upload", async () => {
  const body = new FormData(); body.append("package", $("package").files[0]);
  status("Uploading and validating…");
  const replace = $("replace").value.trim();
  const result = await api(`/assets?kind=${$("kind").value}${replace ? `&replace=${encodeURIComponent(replace)}` : ""}`, {method:"POST",body});
  status(`Published ${result.id} · ${result.version}`); await refresh();
});
action("allow", async () => {await api("/admin/users", {method:"POST",body:{id:$("discord-id").value}}); await refresh();});
action("collect-assets", async () => {
  const result = await api("/admin/assets/gc", {method:"POST",body:{}});
  status(`GC inspected ${result.scanned} entries: ${result.files} files / ${result.bytes} bytes reclaimed; ${result.errors} errors.`);
  await refresh();
});
action("blocks", async () => {
  const form = $("blocks"); const flags = [...form.querySelectorAll("input[type=checkbox]:checked")].reduce((a,b) => a | Number(b.value),0);
  await api(`/blocks/${encodeURIComponent(form.elements.target.value)}`, {method:"PUT",body:{flags}}); status("Privacy rule saved.");
});
$("refresh").onclick = () => refresh().catch(e => status(e.message));
$("logout").onclick = async () => { try { await api("/auth/logout", {method:"POST"}); } catch (_) {} adminSecret = ""; location.reload(); };
load().catch(e => status(`${e.message}. Sign in with an allowlisted Discord account, or unlock administrator access.`));
