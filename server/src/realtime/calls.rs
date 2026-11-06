use crate::db;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
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
}
impl Calls {
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
        match action {
            Action::Invite { user } => {
                anyhow::ensure!(
                    actor != user && online.contains(&user) && !db::blocked(db, actor, &user, 1),
                    "Call invite not permitted"
                );
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
        for user in online {
            notifications.push((user.clone(), self.snapshot(user)));
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
        self.calls.retain(|_, c| {
            c.participants.retain(|u| online.contains(u));
            c.invited
                .retain(|u, t| online.contains(u) && *t > db::now());
            !c.participants.is_empty()
        });
    }
}
