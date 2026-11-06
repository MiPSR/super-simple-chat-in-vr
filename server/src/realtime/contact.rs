use crate::{
    App,
    assets::{Contact, Manifest},
    db,
    world::{Instance, Interaction},
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    target: String,
    #[serde(default = "touch")]
    action: String,
    #[serde(default = "head")]
    segment: String,
    #[serde(default, deserialize_with = "crate::world::deserialize_hand")]
    hand: u8,
}
fn touch() -> String {
    "touch".into()
}
fn head() -> String {
    "head".into()
}
fn allowed(db: &rusqlite::Connection, actor: &str, target: &str) -> bool {
    db::preferences(db, actor).can_touch_others
        && db::preferences(db, target).can_touch_me
        && !db::blocked(db, actor, target, 4)
}
pub fn apply(
    app: &App,
    instance: &mut Instance,
    actor: &str,
    request: Request,
) -> anyhow::Result<Value> {
    anyhow::ensure!(
        request.hand < 2 && request.segment.len() <= 64 && request.target != actor,
        "Invalid contact request"
    );
    let id = format!("{actor}:{}", request.hand);
    if request.action == "release" {
        anyhow::ensure!(
            instance
                .interactions
                .get(&id)
                .is_some_and(|g| g.actor == actor && g.target == request.target),
            "No owned grab"
        );
        instance.interactions.remove(&id);
        instance.revision += 1;
        return Ok(
            json!({"action":"release","actor":actor,"target":request.target,"hand":request.hand}),
        );
    }
    anyhow::ensure!(
        matches!(request.action.as_str(), "touch" | "grab"),
        "Unknown contact action"
    );
    let db = app.db.lock().unwrap();
    anyhow::ensure!(
        allowed(&db, actor, &request.target),
        "Contact permissions denied"
    );
    let target = instance
        .players
        .get(&request.target)
        .ok_or_else(|| anyhow::anyhow!("Target is not in this instance"))?;
    let source = instance
        .players
        .get(actor)
        .ok_or_else(|| anyhow::anyhow!("Actor is not joined"))?;
    let collider = if request.segment == "head" {
        Contact {
            id: "head".into(),
            bone: "Head".into(),
            anchor: "head".into(),
            offset: [0.0; 3],
            radius: 0.18,
            grabbable: false,
        }
    } else {
        let version = target
            .avatar_version
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No avatar collider"))?;
        let text: String = db.query_row(
            "SELECT manifest FROM asset_versions WHERE hash=?",
            [version],
            |r| r.get(0),
        )?;
        let manifest: Manifest = serde_json::from_str(&text)?;
        manifest
            .contacts
            .into_iter()
            .find(|c| c.id == request.segment)
            .ok_or_else(|| anyhow::anyhow!("Unknown collider"))?
    };
    anyhow::ensure!(
        request.action != "grab" || collider.grabbable,
        "This collider is not grabbable"
    );
    let anchor = match collider.anchor.as_str() {
        "left_hand" => 7,
        "right_hand" => 14,
        _ => 0,
    };
    let hand = 7 * (request.hand as usize + 1);
    let q = rapier3d::math::Rotation::from_xyzw(
        target.pose[anchor + 3],
        target.pose[anchor + 4],
        target.pose[anchor + 5],
        target.pose[anchor + 6],
    );
    let q = if q.length_squared() > 0.01 {
        q.normalize()
    } else {
        rapier3d::math::Rotation::IDENTITY
    };
    let offset = q * rapier3d::math::Vector::from(collider.offset);
    let mut center = [0.0; 3];
    for (i, c) in center.iter_mut().enumerate() {
        *c = target.pose[anchor + i] + offset[i];
    }
    if collider.anchor == "hips" {
        center[1] -= 0.7;
    }
    anyhow::ensure!(
        (0..3)
            .map(|i| (source.pose[hand + i] - center[i]).powi(2))
            .sum::<f32>()
            <= (collider.radius + 0.12).powi(2),
        "Contact is out of reach"
    );
    if request.action == "grab" {
        anyhow::ensure!(
            !instance
                .interactions
                .values()
                .any(|g| g.target == request.target
                    && g.segment == request.segment
                    && g.actor != actor),
            "Segment is already grabbed"
        );
        instance.interactions.insert(
            id,
            Interaction {
                actor: actor.into(),
                target: request.target.clone(),
                bone: collider.bone.clone(),
                segment: request.segment.clone(),
                hand: request.hand,
            },
        );
        instance.revision += 1;
    }
    Ok(
        json!({"action":request.action,"actor":actor,"target":request.target,"segment":request.segment,"bone":collider.bone,"hand":request.hand}),
    )
}
pub fn prune(app: &App, instance: &mut Instance) {
    if instance.interactions.is_empty() {
        return;
    }
    let db = app.db.lock().unwrap();
    let before = instance.interactions.len();
    instance.interactions.retain(|_, g| {
        allowed(&db, &g.actor, &g.target)
            && instance.players.contains_key(&g.actor)
            && instance.players.contains_key(&g.target)
    });
    if instance.interactions.len() != before {
        instance.revision += 1;
    }
}
