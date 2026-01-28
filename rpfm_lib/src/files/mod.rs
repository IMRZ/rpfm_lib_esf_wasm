
use getset::*;
use crate::binary::{ReadBytes, WriteBytes};
use crate::error::{Result};
#[allow(dead_code)]pub mod esf;

pub trait Decodeable: Send + Sync {
    fn decode<R: ReadBytes>(data: &mut R, extra_data: &Option<DecodeableExtraData>) -> Result<Self> where Self: Sized;
}

/// A generic trait to implement encoding logic from structured types into anything implementing [WriteBytes].
pub trait Encodeable: Send + Sync {
    fn encode<W: WriteBytes>(&mut self, buffer: &mut W, extra_data: &Option<EncodeableExtraData>) -> Result<()>;
}

#[derive(Clone, Debug, Default, Getters, Setters)]
#[getset(get = "pub", set = "pub")]
pub struct DecodeableExtraData {}

#[derive(Clone, Default, Getters, Setters)]
#[getset(get = "pub", set = "pub")]
pub struct EncodeableExtraData {

    //-----------------------//
    // Configuration toggles //
    //-----------------------//

    /// For recursive encodings in ESF due to compression.
    disable_compression: bool
}
