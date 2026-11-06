//! Version 1 encrypt-then-MAC envelope. HTTPS delivers an independent random
//! per-connection master key; ENet is not claimed to support Godot DTLS.
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub struct Cipher {
    secret: Vec<u8>,
    pub sent: [u64; 3],
    received: [u64; 3],
}
fn key(secret: &[u8], direction: u8, purpose: u8) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(secret);
    h.update([direction, purpose]);
    h.finalize().into()
}
impl Cipher {
    pub fn new(secret: Vec<u8>) -> Self {
        Self {
            secret,
            sent: [0; 3],
            received: [0; 3],
        }
    }
    pub fn seal(&mut self, channel: u8, direction: u8, kind: u8, payload: &[u8]) -> Vec<u8> {
        self.sent[channel as usize] += 1;
        let seq = self.sent[channel as usize];
        let mut iv = [0u8; 16];
        rand::rng().fill_bytes(&mut iv);
        let mut plain = vec![kind];
        plain.extend_from_slice(payload);
        let enc =
            cbc::Encryptor::<aes::Aes256>::new(&key(&self.secret, direction, 0).into(), &iv.into())
                .encrypt_padded_vec_mut::<Pkcs7>(&plain);
        let mut frame = vec![1, channel, direction];
        frame.extend_from_slice(&seq.to_le_bytes());
        frame.extend_from_slice(&iv);
        frame.extend_from_slice(&enc);
        let mut mac = Hmac::<Sha256>::new_from_slice(&key(&self.secret, direction, 1)).unwrap();
        mac.update(&frame);
        frame.extend_from_slice(&mac.finalize().into_bytes());
        frame
    }
    pub fn open(
        &mut self,
        channel: u8,
        direction: u8,
        frame: &[u8],
    ) -> anyhow::Result<(u8, Vec<u8>)> {
        anyhow::ensure!(
            channel < 3
                && frame.len() >= 75
                && frame.len() <= 65536
                && frame[..3] == [1, channel, direction],
            "Invalid envelope"
        );
        let body = &frame[..frame.len() - 32];
        let mut mac = Hmac::<Sha256>::new_from_slice(&key(&self.secret, direction, 1)).unwrap();
        mac.update(body);
        mac.verify_slice(&frame[frame.len() - 32..])
            .map_err(|_| anyhow::anyhow!("Authentication failed"))?;
        let seq = u64::from_le_bytes(frame[3..11].try_into()?);
        anyhow::ensure!(
            seq > self.received[channel as usize],
            "Replay/outdated packet"
        );
        let plain = cbc::Decryptor::<aes::Aes256>::new(
            &key(&self.secret, direction, 0).into(),
            frame[11..27].into(),
        )
        .decrypt_padded_vec_mut::<Pkcs7>(&frame[27..frame.len() - 32])
        .map_err(|_| anyhow::anyhow!("Invalid ciphertext"))?;
        anyhow::ensure!(!plain.is_empty(), "Empty message");
        self.received[channel as usize] = seq;
        Ok((plain[0], plain[1..].to_vec()))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticates_direction_replay_and_tamper() {
        let secret = vec![42; 32];
        let mut a = Cipher::new(secret.clone());
        let mut b = Cipher::new(secret);
        let f = a.seal(0, 0, 3, b"hello");
        assert!(b.open(0, 1, &f).is_err());
        let mut corrupt = f.clone();
        corrupt[30] ^= 1;
        assert!(b.open(0, 0, &corrupt).is_err());
        assert_eq!(b.open(0, 0, &f).unwrap(), (3, b"hello".to_vec()));
        assert!(b.open(0, 0, &f).is_err());
    }
}
