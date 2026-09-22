//! Ubuntu's system-maintained PEM bundle. Reading it does not mutate trust.
use std::{
    fs::File,
    io::{self, Read},
};
pub(super) fn load() -> io::Result<Vec<Vec<u8>>> {
    let mut data = Vec::new();
    File::open("/etc/ssl/certs/ca-certificates.crt")?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut data)?;
    if data.len() > 8 * 1024 * 1024 {
        return Err(io::Error::other("native root bundle exceeds limit"));
    }
    super::pem::decode(&data)
        .ok_or_else(|| io::Error::other("invalid or unsupported native root bundle"))
}
