use crate::db;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone,Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Invite { user: String },
    Accept { call: String },
    Leave,
}
struct Call {
    participants: BTreeSet<String>,
    invited: BTreeMap<String, i64>,
}
#[derive(Default)]
pub struct Calls {
    calls: BTreeMap<String, Call>,
    invite_windows: BTreeMap<String,(i64,u8)>,
    recipient_cooldowns: BTreeMap<String,i64>,
}
impl Calls {
    pub fn creator_action(&mut self, actor: &str, action: Action, online: &BTreeSet<String>, db: &rusqlite::Connection) -> anyhow::Result<Vec<(String,Value)>> {
        anyhow::ensure!(db::preferences(db,actor).allow_creator_calls,"World call controls require your account opt-in");
        anyhow::ensure!(!matches!(action,Action::Accept {..}),"World scripts cannot accept invitations");
        self.action(actor,action,online,db)
    }
    pub fn together(&self, a: &str, b: &str) -> bool {
        self.calls
            .values()
            .any(|c| c.participants.contains(a) && c.participants.contains(b))
    }
    pub fn active(&self, user: &str) -> bool {
        self.calls.values().any(|c| c.participants.contains(user))
    }
    fn current(&self, user: &str) -> Option<String> {
        self.calls
            .iter()
            .find(|(_, c)| c.participants.contains(user))
            .map(|(id, _)| id.clone())
    }
    pub fn snapshot(&self, user: &str) -> Value {
        json!({"event":"state","calls":self.calls.iter().filter(|(_,c)|c.participants.contains(user)).map(|(id,c)|json!({"id":id,"participants":c.participants})).collect::<Vec<_>>()})
    }
    pub fn action(
        &mut self,
        actor: &str,
        action: Action,
        online: &BTreeSet<String>,
        db: &rusqlite::Connection,
    ) -> anyhow::Result<Vec<(String, Value)>> {
        self.calls.retain(|_, c| {
            c.invited.retain(|_, expiry| *expiry > db::now());
            !c.participants.is_empty()
        });
        let mut notifications = Vec::new();
        let mut affected=BTreeSet::from([actor.to_owned()]);
        if let Some(id)=self.current(actor) {
            affected.extend(self.calls[&id].participants.iter().cloned());
        }
        if let Action::Accept {call}=&action
            && let Some(call)=self.calls.get(call) {
            affected.extend(call.participants.iter().cloned());
        }
        let before:BTreeMap<_,_>=affected.iter().map(|user|(user.clone(),self.snapshot(user))).collect();
        match action {
            Action::Invite { user } => {
                anyhow::ensure!(
                    actor != user && online.contains(&user) && !db::blocked(db, actor, &user, 1),
                    "Call invite not permitted"
                );
                let now=db::now();
                self.invite_windows.retain(|_,(started,_)|now-*started<60);
                self.recipient_cooldowns.retain(|_,until|*until>now);
                anyhow::ensure!(self.invite_windows.contains_key(actor) || self.invite_windows.len()<4096,"Call cooldown capacity reached");
                anyhow::ensure!(self.recipient_cooldowns.len()<4096,"Call cooldown capacity reached");
                let window=self.invite_windows.entry(actor.into()).or_insert((now,0));
                if now-window.0>=10 { *window=(now,0); }
                anyhow::ensure!(window.1<4 && !self.recipient_cooldowns.contains_key(&user),"Call invitation rate exceeded");
                window.1+=1;
                self.recipient_cooldowns.insert(user.clone(),now+5);
                anyhow::ensure!(self.calls.len() < 128, "Call capacity reached");
                let id = self
                    .current(actor)
                    .unwrap_or_else(|| db::token()[..16].into());
                let call = self.calls.entry(id.clone()).or_insert_with(|| Call {
                    participants: BTreeSet::from([actor.into()]),
                    invited: BTreeMap::new(),
                });
                anyhow::ensure!(
                    call.participants.len() + call.invited.len() < 8,
                    "Call is full"
                );
                call.invited.insert(user.clone(), db::now() + 60);
                notifications.push((
                    user,
                    json!({"event":"invite","call":id,"from":actor,"expires_in":60}),
                ));
            }
            Action::Accept { call } => {
                let c = self
                    .calls
                    .get(&call)
                    .ok_or_else(|| anyhow::anyhow!("Call no longer exists"))?;
                anyhow::ensure!(
                    c.invited.get(actor).is_some_and(|t| *t > db::now())
                        && c.participants.iter().all(|u| !db::blocked(db, actor, u, 1)),
                    "Call invitation required"
                );
                self.leave(actor);
                let c = self
                    .calls
                    .get_mut(&call)
                    .ok_or_else(|| anyhow::anyhow!("Call no longer exists"))?;
                c.invited.remove(actor);
                c.participants.insert(actor.into());
            }
            Action::Leave => self.leave(actor),
        }
        for user in affected.intersection(online) {
            let state=self.snapshot(user);
            if before.get(user)!=Some(&state) {
                notifications.push((user.clone(),state));
            }
        }
        Ok(notifications)
    }
    pub fn leave(&mut self, user: &str) {
        self.calls.retain(|_, c| {
            c.participants.remove(user);
            c.invited.remove(user);
            !c.participants.is_empty()
        });
    }
    pub fn clean(&mut self, online: &BTreeSet<String>) {
        let now=db::now();
        self.invite_windows.retain(|_,(started,_)|now-*started<60);
        self.recipient_cooldowns.retain(|_,until|*until>now);
        self.calls.retain(|_, c| {
            c.participants.retain(|u| online.contains(u));
            c.invited
                .retain(|u, t| online.contains(u) && *t > db::now());
            !c.participants.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> rusqlite::Connection {
        let db=rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../../migrations/001.sql")).unwrap();
        for user in ["a","b","c","d","e","f"] {
            db.execute("INSERT INTO users(id) VALUES(?)",[user]).unwrap();
        }
        db
    }
    #[test]
    fn invitations_require_acceptance_blocks_and_persistent_cooldowns() {
        let db=db();
        let online=BTreeSet::from_iter(["a","b","c"].map(String::from));
        let mut calls=Calls::default();
        let messages=calls.action("a",Action::Invite {user:"b".into()},&online,&db).unwrap();
        let call=messages.iter().find_map(|(_,message)|message["call"].as_str()).unwrap().to_owned();
        assert!(!calls.together("a","b"));
        assert!(calls.action("c",Action::Accept {call:call.clone()},&online,&db).is_err());
        calls.action("b",Action::Accept {call},&online,&db).unwrap();
        assert!(calls.together("a","b") && !calls.together("a","c"));
        calls.leave("a");
        assert!(calls.action("a",Action::Invite {user:"b".into()},&online,&db).is_err());
        db.execute("INSERT INTO blocks(owner,target,flags) VALUES('c','a',1)",[]).unwrap();
        assert!(calls.action("a",Action::Invite {user:"c".into()},&online,&db).is_err());
    }
    #[test]
    fn one_actor_cannot_flood_unrelated_recipients() {
        let db=db();
        let online=BTreeSet::from_iter(["a","b","c","d","e","f"].map(String::from));
        let mut calls=Calls::default();
        for user in ["b","c","d","e"] {
            calls.action("a",Action::Invite {user:user.into()},&online,&db).unwrap();
        }
        assert!(calls.action("a",Action::Invite {user:"f".into()},&online,&db).is_err());
    }
    #[test]
    fn world_call_controls_require_explicit_account_consent() {
        let db=db();
        let online=BTreeSet::from_iter(["a","b"].map(String::from));
        let mut calls=Calls::default();
        assert!(!db::preferences(&db,"a").allow_creator_calls);
        assert!(calls.creator_action("a",Action::Invite {user:"b".into()},&online,&db).is_err());
        db.execute("UPDATE users SET preferences=? WHERE id='a'",[r#"{"allow_creator_calls":true}"#]).unwrap();
        assert!(calls.creator_action("a",Action::Invite {user:"b".into()},&online,&db).is_ok());
        assert!(calls.creator_action("a",Action::Accept {call:"anything".into()},&online,&db).is_err());
    }
}
