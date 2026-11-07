"use strict";
let adminSecret = "";
let me;
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
async function refresh() {
  const assets = await api(`/assets?q=${encodeURIComponent($("query").value)}`);
  $("assets").replaceChildren(...assets.map(asset => {
    const card = article(asset.name, `${asset.kind} · ${asset.id}\n${asset.description}`);
    if (asset.kind === "world") {
      card.append(button("New public instance", () => grant({ world: asset.id })), button("New invite-only instance", () => grant({ world: asset.id, private: true })));
    }
    if (asset.owner === me.user.id || me.user.admin) card.append(button("Delete", async () => { if (confirm(`Delete ${asset.name}? Running instances keep their version.`)) { await api(`/assets/${asset.id}`, { method: "DELETE" }); await refresh(); } }));
    return card;
  }));
  const instances = await api("/instances");
  $("instances").replaceChildren(...instances.map(i => {
    const a = article(i.id, `${i.players}/${i.capacity} players · ${i.private ? "Invite-only" : "Public"}`);
    a.append(button("Join", () => grant({ instance: i.id })), button("Invite user", async () => {
      const user = prompt("Discord ID to invite"); if (!user) return;
      const invite = await api(`/instances/${i.id}/invite`, { method: "POST", body: { user } });
      status(`Invite: ${JSON.stringify({instance: i.id, ...invite})}`);
    })); return a;
  }));
  if (me.user.admin) {
    const users = await api("/admin/users");
    $("users").replaceChildren(...users.map(u => {
      const a = article(`${u.name || u.id} ${u.enabled ? "" : "(disabled)"}`, `${u.id}: ${u.world_slots} worlds / ${u.avatar_slots} avatars`);
      if (u.id !== "admin") a.append(button("Disable & revoke sessions", async () => { await api(`/admin/users/${u.id}`, { method: "DELETE" }); await refresh(); }));
      a.append(button("Edit quotas", async () => {
        const raw = prompt("Per-user quotas as JSON", JSON.stringify({world_slots:u.world_slots,avatar_slots:u.avatar_slots,world_bytes:u.world_bytes,avatar_bytes:u.avatar_bytes}));
        if (raw) { await api(`/admin/users/${u.id}`, {method:"PUT",body:JSON.parse(raw)}); await refresh(); }
      })); return a;
    }));
    $("health").textContent = JSON.stringify(await api("/admin/health"), null, 2);
  }
}
async function load() {
  me = await api("/me");
  $("account").hidden = false; $("admin").hidden = !me.user.admin;
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
action("preferences", async () => {
  const p = { ...me.preferences };
  for (const key of Object.keys(p)) { const el = $("preferences").elements.namedItem(key); if (!el) continue; p[key] = el.type === "checkbox" ? el.checked : ["quality", "send_divisor", "receive_divisor", "voice_volume", "microphone_gain", "resolution_scale", "texture_lod_bias", "texture_reduction", "radial_hold_seconds"].includes(key) ? Number(el.value) : el.value; }
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
action("blocks", async () => {
  const form = $("blocks"); const flags = [...form.querySelectorAll("input[type=checkbox]:checked")].reduce((a,b) => a | Number(b.value),0);
  await api(`/blocks/${encodeURIComponent(form.elements.target.value)}`, {method:"PUT",body:{flags}}); status("Privacy rule saved.");
});
$("refresh").onclick = () => refresh().catch(e => status(e.message));
$("logout").onclick = async () => { try { await api("/auth/logout", {method:"POST"}); } catch (_) {} adminSecret = ""; location.reload(); };
load().catch(e => status(`${e.message}. Sign in with an allowlisted Discord account, or unlock administrator access.`));
