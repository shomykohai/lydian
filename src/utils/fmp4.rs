use anyhow::{Result, bail};
use axum::body::Bytes;

pub struct Fmp4;

impl Fmp4 {
    const MAGIC: &'static [u8] = b"fLaC";
    pub const STREAMINFO: usize = 34;

    pub fn flac_header(init: &[u8]) -> Result<Bytes> {
        let Some(at) = init.windows(4).position(|window| window == b"dfLa") else {
            bail!("init segment carries no dfLa box, so it is not FLAC in MP4");
        };

        let blocks = at + 8;
        let Some(head) = init.get(blocks..blocks + 4) else {
            bail!("dfLa box is truncated");
        };

        let (last_and_kind, len) = (head[0], u32::from_be_bytes([0, head[1], head[2], head[3]]));

        if last_and_kind & 0x7F != 0 || len as usize != Self::STREAMINFO {
            bail!("dfLa does not start with a STREAMINFO block");
        }

        let Some(body) = init.get(blocks..blocks + 4 + Self::STREAMINFO) else {
            bail!("STREAMINFO block is truncated");
        };

        let mut header = Vec::with_capacity(Self::MAGIC.len() + body.len());
        header.extend_from_slice(Self::MAGIC);
        header.extend_from_slice(body);

        header[Self::MAGIC.len()] = last_and_kind | 0x80;

        Ok(Bytes::from(header))
    }

    pub fn payload_at(fragment: &[u8]) -> Result<usize> {
        let Some(size) = fragment.get(..4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        else {
            bail!("fragment is too short to carry a box header");
        };

        if fragment.get(4..8) != Some(b"moof") {
            bail!("fragment does not begin with a moof box");
        }

        Ok(size as usize + 8)
    }
}
