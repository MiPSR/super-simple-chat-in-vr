"use strict";
const {test} = require("node:test");
const assert = require("node:assert/strict");
const {readFileSync} = require("node:fs");
const {join} = require("node:path");
const {runInNewContext} = require("node:vm");

// Minimal fake DOM: children/name are enough for the dynamically built forms.
function findNamed(root, name) {
  for (const child of root.children) {
    if (child.name === name) return child;
    const nested = findNamed(child, name);
    if (nested) return nested;
  }
  return null;
}
class Element {
  constructor() { this.value = ""; this.hidden = false; this.disabled = false; this.children = []; this.listeners = new Map(); this.elements = {namedItem: name => findNamed(this, name)}; }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  addEventListener(name, fn) { this.listeners.set(name, fn); }
}
async function page(preferences = {}, routes = {}) {
  const html = readFileSync(join(__dirname,"../index.html"),"utf8");
  const elements = new Map();
  for (const match of html.matchAll(/<[^>]+\bid="([^"]+)"[^>]*>/g)) {
    const element = new Element(); element.hidden = /\bhidden(?:\s|>|=)/.test(match[0]); elements.set(match[1],element);
  }
  const requests = [];
  let gc = async () => ({ok:true, json:async () => ({scanned:5,files:2,rows:2,bytes:100,errors:0})});
  const context = {
    document: {getElementById(id) { assert.ok(elements.has(id),`Missing HTML ID: ${id}`); return elements.get(id); },createElement:() => new Element()},
    FormData: class {}, location: {hostname:"example.com",reload() {}},
    async fetch(path, options) {
      requests.push({path,options});
      if (options.headers.Authorization !== "Bearer " + "s".repeat(32)) return {ok:false,statusText:"Forbidden",json:async () => ({error:"Administrator authentication required"})};
      if (path === "/api/v1/admin/assets/gc") return gc();
      if (Object.prototype.hasOwnProperty.call(routes,path)) {
        const route = routes[path];
        if (typeof route === "function") return route(options);
        return {ok:true,json:async () => route};
      }
      const body = path === "/api/v1/me" ? {user:{id:"admin",admin:true},preferences} : path === "/api/v1/admin/health" ? {uploads:{active:0,reserved_bytes:0},creator_vms:{active:0,limit:64},limits:{instances:64,instance_players:64},defaults:{world_slots:4,avatar_slots:8,world_bytes:1000000000,avatar_bytes:100000000}} : [];
      return {ok:true,json:async () => body};
    },
  };
  runInNewContext(readFileSync(join(__dirname,"../app.js"),"utf8"),context,{filename:"app.js"});
  await new Promise(setImmediate);
  async function submit(id,button = new Element()) {
    let prevented = false;
    await elements.get(id).listeners.get("submit")({submitter:button,preventDefault() {prevented = true;}});
    assert.ok(prevented); return button;
  }
  return {elements,requests,submit,setGc(fn) {gc = fn;}};
}

test("admin logs display timestamps and untrusted content as plain text",async () => {
  const line = "<script>alert('not markup')</script>";
  const p = await page({}, {"/api/v1/admin/logs":[{time:0,line}]});
  p.elements.get("admin-secret").value = "s".repeat(32);
  await p.submit("admin-login");
  assert.equal(p.elements.get("logs").textContent,`1970-01-01T00:00:00.000Z ${line}`);
  const request = p.requests.find(request => request.path === "/api/v1/admin/logs");
  assert.equal(request.options.headers.Authorization,"Bearer " + "s".repeat(32));
});

test("administrator policy form preserves types, nested policies and unedited settings",async () => {
  const saved = {tps:60,instance_limit:64,voice_bitrate:24000,network_rates:{minimum_hz:1,maximum_hz:120,automatic:true,suppress_unchanged:false},radio_channels:[1,2],radio_presets:["standard"],audio_effect_max:0.9};
  let received;
  const p = await page({}, {"/api/v1/admin/settings":options => {
    if (options.method === "PUT") {
      received = JSON.parse(options.body);
      return {ok:true,json:async () => ({ok:true,restart_required:true})};
    }
    return {ok:true,json:async () => ({active:saved,saved,restart_required:false})};
  }});
  p.elements.get("server-settings").elements.namedItem = name => findNamed(p.elements.get("policy-fields"),name);
  p.elements.get("admin-secret").value = "s".repeat(32);
  await p.submit("admin-login");
  assert.equal(p.elements.get("server-policy").hidden,false);
  const field = name => p.elements.get("server-settings").elements.namedItem(name);
  field("tps").value = "30";
  field("network_rates.minimum_hz").value = "5";
  field("network_rates.automatic").checked = false;
  field("radio_channels").value = "2, 7";
  field("radio_presets").value = "narrow,weak";
  await p.submit("server-settings");
  assert.equal(received.tps,30);
  assert.equal(received.network_rates.minimum_hz,5);
  assert.equal(received.network_rates.automatic,false);
  assert.deepEqual(received.radio_channels,[2,7]);
  assert.deepEqual(received.radio_presets,["narrow","weak"]);
  assert.equal(received.voice_bitrate,24000);
  assert.match(p.elements.get("policy-status").textContent,/Restart the server/);
});

test("failed policy writes preserve edited controls and display the validation error",async () => {
  const saved = {tps:60};
  const p = await page({}, {"/api/v1/admin/settings":options => options.method === "PUT"
    ? {ok:false,json:async () => ({error:"TPS must be 1..120"})}
    : {ok:true,json:async () => ({saved,active:saved,restart_required:false})}});
  p.elements.get("server-settings").elements.namedItem = name => findNamed(p.elements.get("policy-fields"),name);
  p.elements.get("admin-secret").value = "s".repeat(32);
  await p.submit("admin-login");
  const tps = p.elements.get("server-settings").elements.namedItem("tps");
  tps.value = "999";
  const button = await p.submit("server-settings");
  assert.equal(button.disabled,false);
  assert.equal(tps.value,"999");
  assert.match(p.elements.get("status").textContent,/TPS must/);
});

test("invitation redemption requests a grant with account rates and clears the token on success",async () => {
  const p = await page({send_divisor:2,receive_divisor:4}, {"/api/v1/game-grants":{ticket:"ticket",secret:"secret"}});
  p.elements.get("admin-secret").value = "s".repeat(32);
  await p.submit("admin-login");
  p.elements.get("invite-instance").value = " instance1 ";
  p.elements.get("invite-token").value = " token1 ";
  p.elements.get("grant").select = () => {};
  await p.submit("join-invitation");
  const request = p.requests.find(request => request.path === "/api/v1/game-grants");
  assert.deepEqual(JSON.parse(request.options.body),{instance:"instance1",invitation:"token1",send_divisor:2,receive_divisor:4});
  assert.equal(p.elements.get("invite-token").value,"");
  assert.equal(JSON.parse(p.elements.get("grant").value).host,"example.com");
});

test("failed invitation redemption preserves the token and does not replace the old grant",async () => {
  const p = await page({send_divisor:1,receive_divisor:1}, {"/api/v1/game-grants":() => ({ok:false,json:async () => ({error:"Invitation expired"})})});
  p.elements.get("admin-secret").value = "s".repeat(32);
  await p.submit("admin-login");
  p.elements.get("invite-instance").value = "instance1";
  p.elements.get("invite-token").value = "token1";
  p.elements.get("grant").value = "old grant";
  await p.submit("join-invitation");
  assert.equal(p.elements.get("invite-token").value,"token1");
  assert.equal(p.elements.get("grant").value,"old grant");
  assert.match(p.elements.get("status").textContent,/Invitation expired/);
});

test("comfort controls save numeric values and preserve checkbox preferences",async () => {
  const p = await page({turn_angle:30,move_speed:1.5,shadows:true,snap_turn:true});
  const controls = {turn_angle:new Element(),move_speed:new Element(),shadows:new Element(),snap_turn:new Element()};
  controls.shadows.type = controls.snap_turn.type = "checkbox";
  const radial = Array.from({length:8},() => { const el = new Element(); el.value = "none"; return el; });
  p.elements.get("preferences").elements.namedItem = name => controls[name] || (name.startsWith("radial_action_") ? radial[Number(name.slice(14))] : null);
  p.elements.get("admin-secret").value = "s".repeat(32);
  await p.submit("admin-login");
  controls.turn_angle.value = "45";
  controls.move_speed.value = "0.8";
  controls.shadows.checked = false;
  await p.submit("preferences");
  const request = p.requests.find(request => request.path === "/api/v1/preferences");
  const saved = JSON.parse(request.options.body);
  assert.equal(saved.turn_angle,45);
  assert.equal(saved.move_speed,0.8);
  assert.equal(saved.shadows,false);
  assert.equal(saved.snap_turn,true);
});

test("admin GC form uses the authenticated bounded API and shows its result",async () => {
  const p = await page();
  assert.equal(p.elements.get("admin").hidden,true);
  p.elements.get("admin-secret").value = "s".repeat(32);
  await p.submit("admin-login");
  assert.equal(p.elements.get("admin-secret").value,"");
  assert.equal(p.elements.get("admin").hidden,false);
  assert.equal(p.requests.some(request => request.options.method === "POST"),false,"Page load must not trigger collection");
  let finish;
  p.setGc(() => new Promise(resolve => { finish = resolve; }));
  const button = new Element();
  const pending = p.submit("collect-assets",button);
  assert.equal(button.disabled,true);
  finish({ok:true,json:async () => ({scanned:5,files:2,rows:2,bytes:100,errors:0})});
  await pending;
  assert.equal(button.disabled,false);
  const request = p.requests.find(request => request.path === "/api/v1/admin/assets/gc");
  assert.equal(request.options.method,"POST");
  assert.equal(request.options.headers.Authorization,"Bearer " + "s".repeat(32));
  assert.equal(request.options.credentials,"same-origin");
  assert.deepEqual(JSON.parse(request.options.body),{});
  assert.match(p.elements.get("status").textContent,/2 files \/ 100 bytes reclaimed; 0 errors/);
  assert.match(p.elements.get("health").textContent,/reserved_bytes/);
});

test("GC errors are displayed and do not leave the submit control disabled",async () => {
  const p = await page();
  const button = await p.submit("collect-assets");
  assert.equal(button.disabled,false);
  assert.equal(p.elements.get("admin").hidden,true);
  assert.match(p.elements.get("status").textContent,/Administrator authentication required/);
});

test("draw distance saves as a number without discarding untouched account preferences",async () => {
  const p = await page({draw_distance:250,can_touch_me:true,voice_volume:0.5});
  const distance = new Element(); distance.type = "number";
  const radial = Array.from({length:8},() => { const element = new Element(); element.value = "none"; return element; });
  p.elements.get("preferences").elements.namedItem = name => name === "draw_distance" ? distance : name.startsWith("radial_action_") ? radial[Number(name.slice(14))] : null;
  p.elements.get("admin-secret").value = "s".repeat(32);
  await p.submit("admin-login");
  assert.equal(Number(distance.value),250);
  distance.value = "80";
  await p.submit("preferences");
  const request = p.requests.find(request => request.path === "/api/v1/preferences");
  assert.equal(request.options.method,"PUT");
  const payload = JSON.parse(request.options.body);
  assert.equal(payload.draw_distance,80);
  assert.equal(payload.can_touch_me,true);
  assert.equal(payload.voice_volume,0.5);
  assert.equal(payload.radial_actions.length,8);
});

test("the quota form PUTs numeric quotas for the selected user",async () => {
  const p = await page({},{
    "/api/v1/admin/users":[{id:"42",name:"Alice",enabled:true,world_slots:2,avatar_slots:3,world_bytes:100000,avatar_bytes:200000}],
  });
  p.elements.get("admin-secret").value = "s".repeat(32);
  await p.submit("admin-login");
  const card = p.elements.get("users").children[0];
  const form = card.children.find(child => child.listeners.has("submit"));
  assert.ok(form,"user card must contain a quota form");
  for (const [name,expected] of [["world_slots",2],["avatar_slots",3],["world_bytes",100000],["avatar_bytes",200000]])
    assert.equal(Number(form.elements.namedItem(name).value),expected,`prefilled ${name}`);
  form.elements.namedItem("world_bytes").value = "111111";
  const submitter = new Element();
  let prevented = false;
  await form.listeners.get("submit")({submitter,preventDefault() {prevented = true;}});
  assert.ok(prevented);
  assert.equal(submitter.disabled,false);
  const request = p.requests.find(request => request.path === "/api/v1/admin/users/42");
  assert.equal(request.options.method,"PUT");
  assert.deepEqual(JSON.parse(request.options.body),{world_slots:2,avatar_slots:3,world_bytes:111111,avatar_bytes:200000});
  assert.match(p.elements.get("status").textContent,/Quotas saved/);
});

test("admins can enable and disable asset listings, and a 404 is surfaced",async () => {
  const assets = [
    {id:"7",owner:"admin",kind:"world",name:"Quiet room",description:"",version:"a",enabled:true},
    {id:"8",owner:"admin",kind:"avatar",name:"Reference avatar",description:"",version:"b",enabled:false},
  ];
  const p = await page({},{
    "/api/v1/assets?q=":assets,
    "/api/v1/admin/assets/7/enabled":() => ({ok:false,statusText:"Not Found",json:async () => ({error:"Unknown asset"})}),
    "/api/v1/admin/assets/8/enabled":{ok:true,enabled:true},
  });
  p.elements.get("admin-secret").value = "s".repeat(32);
  await p.submit("admin-login");
  const cards = p.elements.get("assets").children;
  assert.equal(cards.length,2);
  assert.equal(cards[0].children[0].textContent,"Quiet room");
  assert.equal(cards[1].children[0].textContent,"Reference avatar (disabled)");
  const listing = card => card.children.find(child => child.textContent === "Disable listing" || child.textContent === "Enable listing");
  await listing(cards[1]).onclick();
  assert.deepEqual(JSON.parse(p.requests.find(request => request.path === "/api/v1/admin/assets/8/enabled").options.body),{enabled:true});
  await listing(cards[0]).onclick();
  assert.deepEqual(JSON.parse(p.requests.find(request => request.path === "/api/v1/admin/assets/7/enabled").options.body),{enabled:false});
  assert.match(p.elements.get("status").textContent,/Unknown asset/);
});

test("instance cards list roster names with a bounded count",async () => {
  const roster = Array.from({length:15},(_,index) => ({id:`u${index}`,name:`Player ${index}`}));
  const p = await page({},{
    "/api/v1/instances":[{id:"i1",world:"w",version:"v",players:15,capacity:64,private:false,roster}],
  });
  p.elements.get("admin-secret").value = "s".repeat(32);
  await p.submit("admin-login");
  const text = p.elements.get("instances").children[0].children[1].textContent;
  assert.match(text,/15\/64 players/);
  assert.match(text,/Player 0, Player 1, Player 2/);
  assert.match(text,/\+3 more/);
  assert.doesNotMatch(text,/Player 1[234]/);
});

test("the admin panel renders live-instance limits and new-account defaults",async () => {
  const p = await page();
  p.elements.get("admin-secret").value = "s".repeat(32);
  await p.submit("admin-login");
  assert.match(p.elements.get("limits").textContent,/64 instances/);
  assert.match(p.elements.get("limits").textContent,/64 players per instance/);
  assert.match(p.elements.get("defaults").textContent,/4 world slots/);
  assert.match(p.elements.get("defaults").textContent,/8 avatar slots/);
  assert.match(p.elements.get("health").textContent,/reserved_bytes/);
});
