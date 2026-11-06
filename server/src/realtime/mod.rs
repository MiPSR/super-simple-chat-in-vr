mod calls;
mod contact;
mod crypto;
mod radio;
use crate::{App, assets::Manifest, auth, db, world::Join};
pub(crate) use radio::Request as RadioRequest;
use rusty_enet::{Event, Host, HostSettings, Packet, PacketKind, PeerID};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    net::UdpSocket,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Grant {
    pub user: String,
    pub session: String,
    pub name: String,
    pub join: Join,
    pub version: String,
    pub manifest: Manifest,
    pub sources: Vec<String>,
    pub secret: Vec<u8>,
    pub expires: i64,
}
struct Session {
    user: String,
    session: String,
    instance: String,
    cipher: crypto::Cipher,
    send_divisor: u32,
    receive_divisor: u32,
    last_input: Instant,
    last_seen: Instant,
    window: Instant,
    messages: u32,
    voice_frames: u32,
}
fn filter_snapshot_players(snapshot: &mut Value, viewer: &str, db: &rusqlite::Connection) {
    if let Some(players) = snapshot["players"].as_object_mut() {
        players.retain(|user, player| {
            player["audio_allowed"] = json!(!db::blocked(db, viewer, user, 1));
            !db::blocked(db, viewer, user, 8)
        });
    }
}
fn send_bytes(
    peer: &mut rusty_enet::Peer<UdpSocket>,
    session: &mut Session,
    channel: u8,
    kind: u8,
    payload: &[u8],
) {
    let bytes = session.cipher.seal(channel, 1, kind, payload);
    let delivery = if channel == 0 {
        PacketKind::Reliable
    } else {
        PacketKind::Unreliable {
            sequenced: channel != 2,
        }
    };
    let _ = peer.send(channel, &Packet::new(bytes, delivery));
}
fn send(
    peer: &mut rusty_enet::Peer<UdpSocket>,
    session: &mut Session,
    channel: u8,
    kind: u8,
    value: &Value,
) {
    let payload = serde_json::to_vec(value).unwrap();
    if payload.len() > 48000 && channel == 0 {
        let count = payload.len().div_ceil(24000);
        if count > 256 {
            peer.disconnect_later(5);
            return;
        }
        let id = session.cipher.sent[0] + 1;
        for (index, part) in payload.chunks(24000).enumerate() {
            send(
                peer,
                session,
                0,
                10,
                &json!({"id":id,"index":index,"count":count,"kind":kind,"data":hex::encode(part)}),
            );
        }
        return;
    }
    if payload.len() > 60000 {
        return;
    }
    send_bytes(peer, session, channel, kind, &payload);
}
pub fn start(app: Arc<App>) -> anyhow::Result<Arc<AtomicBool>> {
    let socket = UdpSocket::bind(app.config.enet)?;
    let mut host = Host::new(
        socket,
        HostSettings {
            peer_limit: 128,
            channel_limit: 3,
            ..Default::default()
        },
    )?;
    let running = Arc::new(AtomicBool::new(true));
    let alive = running.clone();
    std::thread::Builder::new().name("authoritative-worlds".into()).spawn(move || {
        let mut sessions:HashMap<PeerID,Session>=HashMap::new();
        let mut pending:HashMap<PeerID,Instant>=HashMap::new();
        let mut calls=calls::Calls::default();
        let mut radios=radio::Radios::default();
        let step=Duration::from_secs_f64(1.0/app.config.tps as f64);
        let mut next=Instant::now();
        while alive.load(Ordering::Relaxed) {
            // Bound service work so a packet flood cannot starve the authoritative tick.
            for _ in 0..512 {
                let event=match host.service() {Ok(Some(e))=>e,Ok(None)=>break,Err(e)=>{eprintln!("ENet: {e}");break;}};
                match event {
                    Event::Connect{peer,..}=>{pending.insert(peer.id(),Instant::now());}
                    Event::Disconnect{peer,..}=>{
                        pending.remove(&peer.id());
                        if let Some(s)=sessions.remove(&peer.id()) {app.worlds.lock().unwrap().leave(&s.user);calls.leave(&s.user);}
                    }
                    Event::Receive{peer,channel_id,packet}=>{
                        let id=peer.id();let data=packet.data();
                        if !sessions.contains_key(&id) {
                            if channel_id!=0 || data.len()<64+75 || data.len()>2048 {peer.disconnect_now(1);pending.remove(&id);continue;}
                            let ticket=match std::str::from_utf8(&data[..64]) {Ok(s)=>s,Err(_)=>{peer.disconnect_now(1);continue;}};
                            let grant={app.grants.lock().unwrap().get(ticket).cloned()};
                            let Some(g)=grant else {peer.disconnect_now(1);continue;};
                            if g.expires<=db::now() || !auth::session_valid(&app,&g.user,&g.session) {peer.disconnect_now(1);continue;}
                            let mut cipher=crypto::Cipher::new(g.secret.clone());
                            if !matches!(cipher.open(0,0,&data[64..]),Ok((1,_))) {peer.disconnect_now(1);continue;}
                            app.grants.lock().unwrap().remove(ticket);
                            // One live game connection per account; old connection cannot remove new membership.
                            let old:Vec<_>=sessions.iter().filter(|(_,s)|s.user==g.user).map(|(id,_)|*id).collect();
                            for old in old {sessions.remove(&old);app.worlds.lock().unwrap().leave(&g.user);pending.insert(old,Instant::now()-Duration::from_secs(6));}
                            let result=app.worlds.lock().unwrap().admit(&g.join,&g.user,&g.name,&g.version,g.manifest,g.sources);
                            let instance=match result {Ok(i)=>i,Err(e)=>{let mut s=Session {user:g.user,session:g.session,instance:String::new(),cipher,send_divisor:1,receive_divisor:1,last_input:Instant::now(),last_seen:Instant::now(),window:Instant::now(),messages:0,voice_frames:0};send(peer,&mut s,0,255,&json!({"error":e.to_string()}));peer.disconnect_later(2);continue;}};
                            let mut session=Session {user:g.user,session:g.session,instance:instance.clone(),cipher,send_divisor:g.join.send_divisor,receive_divisor:g.join.receive_divisor,last_input:Instant::now()-step,last_seen:Instant::now(),window:Instant::now(),messages:0,voice_frames:0};
                            let worlds=app.worlds.lock().unwrap();
                            let mut snapshot=worlds.instances[&instance].snapshot(worlds.tick);
                            filter_snapshot_players(&mut snapshot, &session.user, &app.db.lock().unwrap());
                            let user=session.user.clone();
                             send(peer,&mut session,0,2,&json!({"snapshot":snapshot,"user":user,"tps":app.config.tps,"send_hz":app.config.tps/g.join.send_divisor,"receive_hz":app.config.tps/g.join.receive_divisor,"voice_bitrate":app.config.voice_bitrate,"audio_effect_max":app.config.audio_effect_max,"radio_bitrate":app.config.radio_bitrate,"radio_channels":app.config.radio_channels,"radio_policy":{"presets":app.config.radio_presets,"ft8_interval":app.config.radio_ft8_interval,"sstv_interval":app.config.radio_sstv_interval,"noise":app.config.radio_noise,"filter_hz":app.config.radio_filter_hz,"interference":app.config.radio_interference}}));
                            drop(worlds);pending.remove(&id);sessions.insert(id,session);
                        } else if let Some(session)=sessions.get_mut(&id) {
                            if session.window.elapsed()>Duration::from_secs(1) {session.window=Instant::now();session.messages=0;session.voice_frames=0;}
                            session.messages+=1;
                            if session.messages>app.config.tps*3+100 {peer.disconnect_now(3);continue;}
                            let Ok((kind,payload))=session.cipher.open(channel_id,0,data) else {continue;};
                            session.last_seen=Instant::now();
                            if (kind==24 && channel_id==2 || kind==25 && channel_id==0) && payload.len()<=8192 {
                                let deliveries=serde_json::from_slice::<radio::Request>(&payload).ok().filter(|r|r.is_voice()==(kind==24)).map(|request|radios.transmit(&app,&session.instance,&session.user,request));
                                match deliveries {
                                    Some(Ok(messages))=>{for (instance,user,value) in messages {for (id,s) in &mut sessions {if s.instance==instance && s.user==user {send(host.peer_mut(*id),s,if kind==24 {2}else{0},kind,&value);}}}},
                                    Some(Err(error)) if kind==25=>send(peer,session,0,255,&json!({"error":error.to_string()})),
                                    _=>{},
                                }
                            } else if kind==20 && channel_id==2 {
                                if radios.transmitting(&session.user) {continue;}
                                session.voice_frames+=1;
                                let max_bytes=app.config.voice_bitrate as usize/400+64;
                                if session.voice_frames>55 || payload.len()<13 || payload.len()>12+max_bytes || opus::packet::get_nb_samples(&payload[12..],48000).ok()!=Some(960) {continue;}
                                let sender=session.user.clone();let instance=session.instance.clone();
                                let recipients={let worlds=app.worlds.lock().unwrap();let db=app.db.lock().unwrap();let mut routes=HashMap::new();
                                    if let Some(i)=worlds.instances.get(&instance) && let Some(origin)=i.players.get(&sender) {
                                        let mode=db::preferences(&db,&sender).call_mode;
                                        for (other,s) in &sessions {
                                            if s.user==sender || db::blocked(&db,&sender,&s.user,1) {continue;}
                                            let Some(destination)=worlds.instances.get(&s.instance) else {continue;};
                                            let Some(listener)=destination.players.get(&s.user) else {continue;};
                                            let distance=if s.instance==instance {(0..3).map(|j|(origin.pose[j]-listener.pose[j]).powi(2)).sum::<f32>()}else{f32::INFINITY};
                                            let together=calls.together(&sender,&s.user);
                                            let receiver=db::preferences(&db,&s.user).call_mode;
                                            let private=calls.active(&sender) && mode!="spatial";
                                            if private {
                                                if together && i.manifest.allow_voice_bypass && destination.manifest.allow_voice_bypass && receiver!="spatial" {
                                                    routes.insert(*other,if mode=="automatic" && receiver=="automatic" && distance<4.0 {0u8}else{1u8});
                                                } else if together && distance<=900.0 {routes.insert(*other,0);}
                                            } else if distance<=900.0 {routes.insert(*other,0);}
                                        }
                                    } routes};
                                for (other,route) in recipients {if let Some(s)=sessions.get_mut(&other) {let mut frame=vec![route,sender.len() as u8];frame.extend(sender.as_bytes());frame.extend(&payload);send_bytes(host.peer_mut(other),s,2,20,&frame);}}
                            } else if kind==14 && channel_id==1 && payload.len()<=4096 {
                                if let Ok(tracking)=serde_json::from_slice::<std::collections::BTreeMap<String,crate::world::TrackedJoint>>(&payload)
                                    && crate::world::valid_tracking(&tracking)
                                    && let Some(i)=app.worlds.lock().unwrap().instances.get_mut(&session.instance)
                                    && let Some(p)=i.players.get_mut(&session.user) {p.tracking=tracking;}
                            } else if kind==12 && channel_id==1 && payload.len()<=512 {
                                if let Ok(action)=serde_json::from_slice::<crate::world::Action>(&payload)
                                    && let Some(i)=app.worlds.lock().unwrap().instances.get_mut(&session.instance) {let _=i.action(&session.user,action);}
                            } else if kind==3 && channel_id==1 {
                                if payload.len()!=84 || session.last_input.elapsed()<step*session.send_divisor {continue;}
                                let pose:Vec<f32>=payload.as_chunks::<4>().0.iter().map(|s|f32::from_le_bytes(*s)).collect();
                                if !pose.iter().all(|v|v.is_finite() && v.abs()<100000.0) {continue;}
                                let mut worlds=app.worlds.lock().unwrap();
                                if let Some(i)=worlds.instances.get_mut(&session.instance) && let Some(p)=i.players.get_mut(&session.user) {p.pose=pose;}
                                session.last_input=Instant::now();
                            } else if channel_id==0 {
                                if payload.len()>8192 {continue;}
                                let Ok(value)=serde_json::from_slice::<Value>(&payload) else {continue;};
                                match kind {
                                    4=>{ // Text is not broadcast to anyone who blocks either party.
                                        let Some(text)=value["text"].as_str().filter(|s|!s.is_empty() && s.len()<=512) else {continue;};
                                        let sender=session.user.clone();
                                        let instance=session.instance.clone();
                                        let recipients:Vec<_>=sessions.iter().filter(|(_,s)|s.instance==instance).map(|(id,s)|(*id,s.user.clone())).collect();
                                        for (other,user) in recipients {if !db::blocked(&app.db.lock().unwrap(),&sender,&user,2) && let Some(s)=sessions.get_mut(&other) {send(host.peer_mut(other),s,0,4,&json!({"user":sender,"text":text}));}}
                                    }
                                    5=>{if let Some(target)=value["target"].as_str() && let Some(i)=app.worlds.lock().unwrap().instances.get_mut(&session.instance) && let Err(e)=i.interact(&session.user,target) {send(peer,session,0,255,&json!({"error":e.to_string()}));}}
                                    6=>{
                                        let sender=session.user.clone();let instance=session.instance.clone();
                                        let event=serde_json::from_value::<contact::Request>(value).ok().and_then(|request| {
                                            let mut worlds=app.worlds.lock().unwrap();let i=worlds.instances.get_mut(&instance)?;
                                            let event=contact::apply(&app,i,&sender,request).ok()?;
                                            i.dispatch(event["action"].as_str().unwrap_or("touch"),event.clone());
                                            Some(event)
                                        });
                                        if let Some(event)=event {for (other,s) in &mut sessions {if s.instance==instance && !db::blocked(&app.db.lock().unwrap(),&s.user,&sender,16) && !db::blocked(&app.db.lock().unwrap(),&s.user,event["target"].as_str().unwrap_or(""),16) {send(host.peer_mut(*other),s,0,6,&event);}}}
                                    }
                                    7=>{let mut worlds=app.worlds.lock().unwrap();if let Some(i)=worlds.instances.get_mut(&session.instance) && let Some(p)=i.players.get_mut(&session.user) {p.sleeping=value["sleeping"].as_bool().unwrap_or(false);i.revision+=1;}}
                                    8=>{let valid=value["avatar"].as_str().and_then(|a| app.db.lock().unwrap().query_row("SELECT id,version FROM assets WHERE id=? AND owner=? AND kind='avatar' AND enabled=1",rusqlite::params![a,session.user],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).ok());let mut worlds=app.worlds.lock().unwrap();if let Some(i)=worlds.instances.get_mut(&session.instance) && let Some(p)=i.players.get_mut(&session.user) {p.avatar=valid.as_ref().map(|a|a.0.clone());p.avatar_version=valid.map(|a|a.1);p.avatar_actions.clear();i.revision+=1;}}
                                    9=>{peer.disconnect_later(0);}
                                    12=>{if let Ok(action)=serde_json::from_value::<crate::world::Action>(value)
                                        && let Some(i)=app.worlds.lock().unwrap().instances.get_mut(&session.instance)
                                        && let Err(e)=i.action(&session.user,action) {send(peer,session,0,255,&json!({"error":e.to_string()}));}}
                                    13=>{
                                        let Some(name)=value["name"].as_str().filter(|s|s.len()<=64) else {continue;};
                                        let Some(weight)=value["weight"].as_f64().filter(|v|v.is_finite() && (0.0..=1.0).contains(v)) else {continue;};
                                        let mut worlds=app.worlds.lock().unwrap();
                                        if let Some(i)=worlds.instances.get_mut(&session.instance) && let Some(p)=i.players.get_mut(&session.user)
                                            && let Some(version)=&p.avatar_version {
                                            let manifest=app.db.lock().unwrap().query_row("SELECT manifest FROM asset_versions WHERE hash=?",[version],|r|r.get::<_,String>(0)).ok().and_then(|s|serde_json::from_str::<Manifest>(&s).ok());
                                            if manifest.is_some_and(|m|m.actions.iter().any(|a|a==name)) {p.avatar_actions.insert(name.into(),weight as f32);i.revision+=1;}
                                        }
                                    }
                                    21=>{
                                        let actor=session.user.clone();
                                        let online=sessions.values().map(|s|s.user.clone()).collect();
                                        let messages=serde_json::from_value::<calls::Action>(value).ok().and_then(|action|calls.action(&actor,action,&online,&app.db.lock().unwrap()).ok());
                                        if let Some(messages)=messages {for (user,value) in messages {for (id,s) in &mut sessions {if s.user==user {send(host.peer_mut(*id),s,0,21,&value);}}}}
                                    }
                                    22=>{
                                        let result=serde_json::from_value::<crate::world::media::Request>(value).map_err(anyhow::Error::from).and_then(|request| {
                                            let mut worlds=app.worlds.lock().unwrap();
                                            let instance=worlds.instances.get_mut(&session.instance).ok_or_else(||anyhow::anyhow!("Instance ended"))?;
                                            instance.media_command(&session.user,request)
                                        });
                                        if let Err(error)=result {send(peer,session,0,255,&json!({"error":error.to_string()}));}
                                    }
                                    23=>{
                                        if let Some(sent)=value["sent"].as_f64() && sent.is_finite() {
                                            send(peer,session,0,23,&json!({"sent":sent,"server":crate::world::media::now()}));
                                        }
                                    }
                                    _=>{}
                                }
                            }
                        }
                    }
                }
            }
            if Instant::now()>=next {
                next+=step;if next<Instant::now()-step*4 {next=Instant::now()+step;}
                let expired:Vec<_>=sessions.iter().filter(|(_,s)|s.last_seen.elapsed()>Duration::from_secs(15) || !auth::session_valid(&app,&s.user,&s.session)).map(|(id,_)|*id).collect();
                for id in expired {if let Some(s)=sessions.remove(&id) {app.worlds.lock().unwrap().leave(&s.user);calls.leave(&s.user);}host.peer_mut(id).disconnect_now(4);}
                calls.clean(&sessions.values().map(|s|s.user.clone()).collect());
                radios.clean(&sessions.values().map(|s|s.user.clone()).collect());
                pending.retain(|id,at|{if at.elapsed()>Duration::from_secs(5) {host.peer_mut(*id).disconnect_now(4);false}else{true}});
                let mut worlds=app.worlds.lock().unwrap();worlds.step();
                for i in worlds.instances.values_mut() {contact::prune(&app,i);}
                for (id,s) in &mut sessions {
                    if worlds.tick.is_multiple_of(s.receive_divisor as u64)
                        && let Some(i)=worlds.instances.get(&s.instance) {
                            let db=app.db.lock().unwrap();
                            let players:Vec<_>=i.players.values().filter(|p|!db::blocked(&db,&s.user,&p.id,8)).map(|p| {
                                let mut value=json!(p);value["audio_allowed"]=json!(!db::blocked(&db,&s.user,&p.id,1));value
                            }).collect();
                            let transforms:Vec<_>=i.entities.values().map(|e|json!({"id":e.id,"position":e.position,"rotation":e.rotation})).collect();
                             let media:std::collections::BTreeMap<_,_>=i.media.iter().map(|(id,m)|(id,m.snapshot())).collect();
                             send(host.peer_mut(*id),s,1,3,&json!({"tick":worlds.tick,"revision":i.revision,"players":players,"entities":transforms,"media":media}));
                            // Reliable graph/state updates are independent of the negotiated pose rate.
                    }
                }
                // Reliable shared state: at most once per tick, never downsampled.
                static LAST:std::sync::OnceLock<std::sync::Mutex<HashMap<PeerID,u64>>>=std::sync::OnceLock::new();
                let mut revisions=LAST.get_or_init(Default::default).lock().unwrap();
                revisions.retain(|id,_|sessions.contains_key(id));
                for (id,s) in &mut sessions {if let Some(i)=worlds.instances.get(&s.instance) && revisions.get(id)!=Some(&i.revision) {
                    let mut snapshot=i.snapshot(worlds.tick);
                    filter_snapshot_players(&mut snapshot, &s.user, &app.db.lock().unwrap());
                    send(host.peer_mut(*id),s,0,2,&json!({"snapshot":snapshot}));revisions.insert(*id,i.revision);
                }}
                for (id,s) in &mut sessions {if let Some(i)=worlds.instances.get(&s.instance) && !i.events.is_empty() {
                    send(host.peer_mut(*id),s,0,11,&json!({"tick":worlds.tick,"events":i.events}));
                }}
                for i in worlds.instances.values_mut() {i.events.clear();}
                let radio_requests:Vec<_>=worlds.instances.iter_mut().flat_map(|(id,instance)| {
                    std::mem::take(&mut instance.radio_requests).into_iter().map(move |(actor,request)|(id.clone(),actor,request))
                }).collect();
                drop(worlds);
                // Radio routing takes its own worlds lock and uses the same
                // membership/reach/channel/block/cooldown checks as the phone.
                for (instance,actor,request) in radio_requests {
                    match radios.transmit(&app,&instance,&actor,request) {
                        Ok(messages)=>for (destination,user,value) in messages {
                            for (id,s) in &mut sessions {if s.instance==destination && s.user==user {send(host.peer_mut(*id),s,0,25,&value);}}
                        },
                        Err(error)=>for (id,s) in &mut sessions {if s.instance==instance && s.user==actor {send(host.peer_mut(*id),s,0,255,&json!({"error":error.to_string()}));}}
                    }
                }
                host.flush();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    })?;
    Ok(running)
}
