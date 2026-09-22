//! FIPS 197 forward AES for the 128/256-bit TLS key sizes. GCM needs no inverse.
use super::{
    aes_substitution::{double, substitute},
    secret::erase,
};

pub(crate) struct Block(pub [u8; 16]);
impl Drop for Block {
    fn drop(&mut self) {
        erase(&mut self.0);
    }
}

pub(crate) struct Aes {
    schedule: [u8; 240],
    rounds: usize,
}

impl Aes {
    pub(crate) fn new(key: &[u8]) -> Option<Self> {
        let rounds = match key.len() {
            16 => 10,
            32 => 14,
            _ => return None,
        };
        let words = key.len() / 4;
        let mut cipher = Self {
            schedule: [0; 240],
            rounds,
        };
        cipher.schedule[..key.len()].copy_from_slice(key);
        let mut constant = 1;
        for index in words..4 * (rounds + 1) {
            let mut previous: [u8; 4] = cipher.schedule[4 * (index - 1)..4 * index]
                .try_into()
                .expect("schedule word");
            if index % words == 0 {
                previous.rotate_left(1);
                for byte in &mut previous {
                    *byte = substitute(*byte);
                }
                previous[0] ^= constant;
                constant = double(constant);
            } else if words == 8 && index % words == 4 {
                for byte in &mut previous {
                    *byte = substitute(*byte);
                }
            }
            for (byte, value) in previous.iter().enumerate() {
                cipher.schedule[4 * index + byte] =
                    cipher.schedule[4 * (index - words) + byte] ^ value;
            }
            erase(&mut previous);
        }
        Some(cipher)
    }

    pub(crate) fn encrypt(&self, input: &[u8; 16]) -> Block {
        let mut state = Block(*input);
        self.add_key(&mut state.0, 0);
        for round in 1..=self.rounds {
            let mut shifted = Block([0; 16]);
            for column in 0..4 {
                for row in 0..4 {
                    shifted.0[4 * column + row] =
                        substitute(state.0[4 * ((column + row) % 4) + row]);
                }
            }
            state.0.copy_from_slice(&shifted.0);
            if round != self.rounds {
                for column in state.0.chunks_exact_mut(4) {
                    let all = column[0] ^ column[1] ^ column[2] ^ column[3];
                    let first = column[0];
                    column[0] ^= all ^ double(column[0] ^ column[1]);
                    column[1] ^= all ^ double(column[1] ^ column[2]);
                    column[2] ^= all ^ double(column[2] ^ column[3]);
                    column[3] ^= all ^ double(column[3] ^ first);
                }
            }
            self.add_key(&mut state.0, round);
        }
        state
    }

    fn add_key(&self, state: &mut [u8; 16], round: usize) {
        for (byte, key) in state
            .iter_mut()
            .zip(&self.schedule[16 * round..16 * (round + 1)])
        {
            *byte ^= key;
        }
    }
}

impl Drop for Aes {
    fn drop(&mut self) {
        erase(&mut self.schedule);
    }
}
