//! Hash choices admitted by the initial certificate verification profile.
use super::{sha256::Sha256, sha384::Sha384};
#[derive(Clone, Copy)]
pub(in crate::tls) enum Hash {
    Sha256,
    Sha384,
}
impl Hash {
    pub fn bytes(self) -> usize {
        match self {
            Self::Sha256 => 32,
            Self::Sha384 => 48,
        }
    }
    pub fn digest(self, fragments: &[&[u8]]) -> Vec<u8> {
        match self {
            Self::Sha256 => {
                let mut hash = Sha256::new();
                for part in fragments {
                    hash.update(part);
                }
                hash.finish().to_vec()
            }
            Self::Sha384 => {
                let mut hash = Sha384::new();
                for part in fragments {
                    hash.update(part);
                }
                hash.finish().to_vec()
            }
        }
    }
}
