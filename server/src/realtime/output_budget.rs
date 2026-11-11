//! Reservations live in ENet-owned packet data, not an application ACK ledger.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const PEER_LIMIT: usize = 4 * 1024 * 1024;
const GLOBAL_LIMIT: usize = 64 * 1024 * 1024;
static GLOBAL: AtomicUsize = AtomicUsize::new(0);

pub fn usage() -> serde_json::Value {
    serde_json::json!({"reserved_bytes":GLOBAL.load(Ordering::Relaxed),"peer_limit_bytes":PEER_LIMIT,"process_limit_bytes":GLOBAL_LIMIT})
}

#[derive(Default)]
pub struct Budget(Arc<AtomicUsize>);

pub struct Bytes {
    data: Vec<u8>,
    peer: Arc<AtomicUsize>,
    charge: usize,
}

impl Budget {
    pub fn packet(&self, data: Vec<u8>, mtu: usize) -> Option<Box<Bytes>> {
        // Charge conservative command/fragment overhead as well as retained
        // allocation capacity. ENet's minimum MTU is 576; subtracting 128
        // leaves less payload than its headers actually require.
        let fragment_payload = mtu.max(576).saturating_sub(128);
        let charge = data
            .capacity()
            .checked_add(data.len().div_ceil(fragment_payload).checked_mul(256)?)?
            .checked_add(256)?;
        reserve(&self.0, charge, PEER_LIMIT)?;
        if reserve(&GLOBAL, charge, GLOBAL_LIMIT).is_none() {
            self.0.fetch_sub(charge, Ordering::Relaxed);
            return None;
        }
        Some(Box::new(Bytes {
            data,
            peer: self.0.clone(),
            charge,
        }))
    }
}

fn reserve(counter: &AtomicUsize, charge: usize, limit: usize) -> Option<()> {
    counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
            used.checked_add(charge).filter(|next| *next <= limit)
        })
        .ok()
        .map(|_| ())
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl Drop for Bytes {
    fn drop(&mut self) {
        self.peer.fetch_sub(self.charge, Ordering::Relaxed);
        GLOBAL.fetch_sub(self.charge, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_enet::Packet;

    #[test]
    fn health_usage_reports_live_reservations_and_limits() {
        let budget = Budget::default();
        let packet = Packet::reliable(budget.packet(vec![0; 1024], 576).unwrap());
        let state = usage();
        assert_eq!(state["peer_limit_bytes"].as_u64(), Some(PEER_LIMIT as u64));
        assert_eq!(
            state["process_limit_bytes"].as_u64(),
            Some(GLOBAL_LIMIT as u64)
        );
        let used = state["reserved_bytes"].as_u64().unwrap();
        assert!(used >= budget.0.load(Ordering::Relaxed) as u64 && used <= GLOBAL_LIMIT as u64);
        drop(packet);
        assert_eq!(budget.0.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn packet_ownership_and_clones_hold_and_release_reservations() {
        let budget = Budget::default();
        let packet = Packet::reliable(budget.packet(vec![7; 65536], 576).unwrap());
        let retained = budget.0.load(Ordering::Relaxed);
        assert!(retained > 65536);
        let clone = packet.clone();
        drop(packet);
        assert_eq!(budget.0.load(Ordering::Relaxed), retained);
        assert_eq!(clone.data(), &vec![7; 65536]);
        drop(clone);
        assert_eq!(budget.0.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn slow_peer_admission_stops_at_limit_and_recovers_after_release() {
        let budget = Budget::default();
        let mut packets = Vec::new();
        while let Some(bytes) = budget.packet(vec![0; 60000], 576) {
            packets.push(Packet::reliable(bytes));
        }
        assert!(!packets.is_empty());
        assert!(budget.0.load(Ordering::Relaxed) <= PEER_LIMIT);
        assert!(budget.packet(vec![0; 60000], 576).is_none());
        drop(packets);
        assert_eq!(budget.0.load(Ordering::Relaxed), 0);
        assert!(budget.packet(vec![0; 60000], 576).is_some());
    }

    #[test]
    fn actual_enet_unacknowledged_queue_releases_on_disconnect() {
        use rusty_enet::{Event, Host, HostSettings};
        use std::{
            net::UdpSocket,
            time::{Duration, Instant},
        };
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        let settings = || HostSettings {
            peer_limit: 1,
            channel_limit: 1,
            ..Default::default()
        };
        let mut server = Host::new(socket, settings()).unwrap();
        let mut client = Host::new(UdpSocket::bind("127.0.0.1:0").unwrap(), settings()).unwrap();
        let id = client.connect(address, 1, 0).unwrap().id();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut connected = false;
        while Instant::now() < deadline && !connected {
            while server.service().unwrap().is_some() {}
            while let Some(event) = client.service().unwrap() {
                if matches!(event, Event::Connect { .. }) {
                    connected = true;
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(connected);
        let budget = Budget::default();
        // Do not service the receiver: none of these data commands can retire.
        while let Some(bytes) = budget.packet(vec![0; 60000], client.peer(id).mtu() as usize) {
            client
                .peer_mut(id)
                .send(0, &Packet::reliable(bytes))
                .unwrap();
        }
        assert!(budget.0.load(Ordering::Relaxed) > 0);
        client.peer_mut(id).disconnect_now(5);
        assert_eq!(budget.0.load(Ordering::Relaxed), 0);
    }
}
