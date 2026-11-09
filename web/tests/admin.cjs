"use strict";
const {test} = require("node:test");
const assert = require("node:assert/strict");
const {readFileSync} = require("node:fs");
const {join} = require("node:path");
const {runInNewContext} = require("node:vm");

class Element {
  constructor() { this.value = ""; this.hidden = false; this.disabled = false; this.children = []; this.listeners = new Map(); this.elements = {namedItem: () => null}; }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  addEventListener(name, fn) { this.listeners.set(name, fn); }
}
async function page() {
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
      const body = path === "/api/v1/me" ? {user:{id:"admin",admin:true},preferences:{}} : path === "/api/v1/admin/health" ? {uploads:{active:0,reserved_bytes:0},creator_vms:{active:0,limit:64}} : [];
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
