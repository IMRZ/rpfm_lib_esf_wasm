use std::io::SeekFrom;

use std::io::{Error, ErrorKind, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::binary::{ReadBytes, WriteBytes};
use crate::error::{RLibError, Result};

use super::reader::{self, NodeMeta};
use super::*;

pub const ABCE_CODEC_ID: u32 = 0xABCE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbceHeader {
    pub id: u32,
    pub unknown_1: u32,
    pub edit_time: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbceCodec {
    id: u32,
}

impl Default for AbceCodec {
    fn default() -> Self {
        Self::new(ABCE_CODEC_ID)
    }
}

impl AbceCodec {
    pub const fn new(id: u32) -> Self {
        Self { id }
    }

    pub const fn id(&self) -> u32 {
        self.id
    }

    pub fn read_header<R: ReadBytes>(&self, reader: &mut R) -> Result<AbceHeader> {
        Ok(AbceHeader {
            id: reader.read_u32()?,
            unknown_1: reader.read_u32()?,
            edit_time: reader.read_u32()?,
        })
    }

    pub fn write_header<W: WriteBytes>(&self, writer: &mut W) -> Result<()> {
        writer.write_u32(self.id)?;
        writer.write_u32(0)?;
        writer.write_u32(Self::get_timestamp(SystemTime::now())?)?;
        Ok(())
    }

    pub fn get_time(stamp: u32) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(u64::from(stamp))
    }

    pub fn get_timestamp(time: SystemTime) -> Result<u32> {
        let elapsed = time
            .duration_since(UNIX_EPOCH)
            .map_err(|error| RLibError::IOError(Error::new(ErrorKind::InvalidInput, error)))?;

        u32::try_from(elapsed.as_secs()).map_err(|_| {
            RLibError::IOError(Error::new(
                ErrorKind::InvalidInput,
                "timestamp exceeds the ABCE header range",
            ))
        })
    }
}

impl ESF {
    pub(crate) fn read_ceab<R: ReadBytes>(&mut self, data: &mut R) -> Result<()> {
        self.unknown_1 = data.read_u32()?;
        self.creation_date = data.read_u32()?;
        let record_names_offset = data.read_u32()?;
        let nodes_offset = data.stream_position()?;

        data.seek(SeekFrom::Start(record_names_offset as u64))?;
        let names_count = data.read_i16()?;
        if names_count < 0 {
            return Err(RLibError::DecodingMismatchSizeError(
                0,
                names_count.unsigned_abs() as usize,
            ));
        }

        let mut record_names = Vec::with_capacity(names_count as usize);
        for _ in 0..names_count {
            record_names.push(data.read_sized_string_u8()?);
        }

        // The record names are the last thing in a well-formed file, but real files can carry
        // zero padding after them.
        let names_end = data.stream_position()?;
        let data_len = data.len()?;
        if names_end != data_len && !ESF::only_padding_remains(data)? {
            return Err(RLibError::DecodingMismatchSizeError(
                data_len as usize,
                names_end as usize,
            ));
        }

        data.seek(SeekFrom::Start(nodes_offset))?;
        self.root_node = read_node(data, &record_names)?;

        let nodes_end = data.stream_position()?;
        if nodes_end != record_names_offset as u64 {
            return Err(RLibError::DecodingMismatchSizeError(
                record_names_offset as usize,
                nodes_end as usize,
            ));
        }

        Ok(())
    }

    pub(crate) fn save_ceab<W: WriteBytes>(
        &mut self,
        buffer: &mut W,
        _extra_data: &Option<EncodeableExtraData>,
    ) -> Result<()> {
        buffer.write_u32(0)?;
        buffer.write_u32(AbceCodec::get_timestamp(SystemTime::now())?)?;

        let mut record_names = Vec::new();
        let mut strings_utf8 = Vec::new();
        let mut strings_utf16 = Vec::new();
        Self::read_string_from_node(
            &self.root_node,
            &mut record_names,
            &mut strings_utf8,
            &mut strings_utf16,
        );

        // CEAB stores its strings inline, so only the record names are needed here. The string
        // lists are still collected because the shared helper returns all three together.
        let mut nodes_data = Vec::new();
        save_node(&mut nodes_data, 16, &self.root_node, &record_names)?;

        let mut names_data = Vec::new();
        names_data.write_i16(record_names.len() as i16)?;
        for name in &record_names {
            names_data.write_sized_string_u8(name)?;
        }

        buffer.write_u32((12 + nodes_data.len() + 4) as u32)?;
        buffer.write_all(&nodes_data)?;
        buffer.write_all(&names_data)?;

        Ok(())
    }
}

//---------------------------------------------------------------------------//
//                          CEAB node decoding/encoding
//---------------------------------------------------------------------------//
//
// CEAB gets its own node walker rather than sharing one with the other formats, because it
// differs in ways a shared implementation hides:
//
//   * Every size and count is 32-bit, and a size is an *absolute end offset*, not a length
//     relative to the current position.
//   * Record info is always a `u16` name index followed by a `u8` version. There is no
//     packed 2-byte form, and the root node is not special in any way.
//   * Strings are stored inline in the node, so a CEAB file has no string tables at all.
//   * A record holding several entries uses the `0x81` marker. A plain record uses `0x80`,
//     and its end offset also delimits its single group of children.

/// Marker of a plain record.
const RECORD: u8 = 0x80;

/// Marker of a record holding several entries, each with its own end offset.
const RECORD_BLOCK: u8 = 0x81;

/// Reads a CEAB node, and everything below it.
pub(crate) fn read_node<R: ReadBytes>(data: &mut R, record_names: &[String]) -> Result<NodeType> {
    let next_byte = data.read_u8()?;

    if next_byte != RECORD && next_byte != RECORD_BLOCK {
        return read_leaf(data, next_byte);
    }

    let is_block = next_byte == RECORD_BLOCK;
    let name_index = data.read_u16()?;
    let version = data.read_u8()?;
    let name = record_names
        .get(name_index as usize)
        .ok_or(RLibError::DecodingESFRecordNameNotFound(name_index))?
        .clone();

    let final_block_offset = data.read_u32()? as u64;
    let group_count = if is_block { data.read_u32()? } else { 1 };

    let mut children = Vec::with_capacity(group_count as usize);
    for _ in 0..group_count {
        let group_end = if is_block {
            data.read_u32()? as u64
        } else {
            final_block_offset
        };

        let mut nodes = Vec::new();
        while data.stream_position()? < group_end {
            nodes.push(read_node(data, record_names)?);
        }

        let position = data.stream_position()?;
        if position != group_end {
            return Err(RLibError::DecodingMismatchSizeError(group_end as usize, position as usize));
        }

        children.push(nodes);
    }

    let position = data.stream_position()?;
    if position != final_block_offset {
        return Err(RLibError::DecodingMismatchSizeError(final_block_offset as usize, position as usize));
    }

    let record_flags = if is_block {
        RecordNodeFlags::IS_RECORD_NODE | RecordNodeFlags::HAS_NESTED_BLOCKS
    } else {
        RecordNodeFlags::IS_RECORD_NODE
    };

    Ok(NodeType::Record(Box::new(RecordNode {
        record_flags,
        version,
        name,
        children,
    })))
}

/// Reads a CEAB leaf node.
fn read_leaf<R: ReadBytes>(data: &mut R, next_byte: u8) -> Result<NodeType> {
    match next_byte {
        INVALID => Err(RLibError::DecodingESFUnsupportedDataType(next_byte)),

        // CEAB stores its strings inline instead of referencing a shared table.
        UTF16 => Ok(NodeType::Utf16(data.read_sized_string_u16()?)),
        ASCII => Ok(NodeType::Ascii(data.read_sized_string_u8()?)),

        UTF16_ARRAY => {
            let end_offset = data.read_u32()? as u64;
            let mut values = Vec::new();
            while data.stream_position()? < end_offset {
                values.push(data.read_sized_string_u16()?);
            }
            Ok(NodeType::Utf16Array(values))
        }
        ASCII_ARRAY => {
            let end_offset = data.read_u32()? as u64;
            let mut values = Vec::new();
            while data.stream_position()? < end_offset {
                values.push(data.read_sized_string_u8()?);
            }
            Ok(NodeType::AsciiArray(values))
        }

        UNKNOWN_21 => Ok(NodeType::Unknown21(data.read_u32()?)),
        UNKNOWN_25 => Ok(NodeType::Unknown25(data.read_u32()?)),

        _ => {
            // Arrays are marked by an absolute end offset, primitives are marked by nothing.
            if ESF::is_primitive_array(next_byte) {
                let end_offset = data.read_u32()? as u64;
                return match ESF::read_array_items(data, next_byte, end_offset)? {
                    Some(node) => Ok(node),
                    None => Err(RLibError::DecodingESFUnsupportedDataType(next_byte)),
                };
            }

            match ESF::read_primitive(data, next_byte)? {
                Some(node) => Ok(node),
                None => Err(RLibError::DecodingESFUnsupportedDataType(next_byte)),
            }
        }
    }
}

/// Writes a CEAB node, and everything below it, into `buffer`.
///
/// `base` is the absolute offset `buffer[0]` will end up at. CEAB writes sizes as absolute
/// offsets, so the writer has to know where it stands in the final file.
pub(crate) fn save_node(
    buffer: &mut Vec<u8>,
    base: u64,
    node: &NodeType,
    record_names: &[String],
) -> Result<()> {
    if let NodeType::Record(record) = node {
        let is_block = record.children().len() > 1;
        let name_index = record_names
            .iter()
            .position(|name| name == record.name())
            .ok_or_else(|| RLibError::EncodingESFStringNotFound(record.name().to_owned()))?;

        buffer.write_u8(if is_block { RECORD_BLOCK } else { RECORD })?;
        buffer.write_u16(name_index as u16)?;
        buffer.write_u8(*record.version())?;

        // The end offset is only known once the children are written, so leave a hole.
        let block_offset_at = buffer.len();
        buffer.write_u32(0)?;

        if is_block {
            buffer.write_u32(record.children().len() as u32)?;
        }

        for group in record.children() {
            let entry_offset_at = if is_block {
                let at = buffer.len();
                buffer.write_u32(0)?;
                Some(at)
            } else {
                None
            };

            for child in group {
                save_node(buffer, base, child, record_names)?;
            }

            if let Some(at) = entry_offset_at {
                let end = (base + buffer.len() as u64) as u32;
                buffer[at..at + 4].copy_from_slice(&end.to_le_bytes());
            }
        }

        let end = (base + buffer.len() as u64) as u32;
        buffer[block_offset_at..block_offset_at + 4].copy_from_slice(&end.to_le_bytes());
        return Ok(());
    }

    if let Some((marker, items)) = ESF::encode_array_items(node)? {
        buffer.write_u8(marker)?;
        let size_at = buffer.len();
        buffer.write_u32(0)?;
        buffer.write_all(&items)?;

        let end = (base + buffer.len() as u64) as u32;
        buffer[size_at..size_at + 4].copy_from_slice(&end.to_le_bytes());
        return Ok(());
    }

    let size_at = match node {
        NodeType::Utf16(value) => { buffer.write_u8(UTF16)?; buffer.write_sized_string_u16(value)?; None }
        NodeType::Ascii(value) => { buffer.write_u8(ASCII)?; buffer.write_sized_string_u8(value)?; None }
        NodeType::Unknown21(value) => { buffer.write_u8(UNKNOWN_21)?; buffer.write_u32(*value)?; None }
        NodeType::Unknown25(value) => { buffer.write_u8(UNKNOWN_25)?; buffer.write_u32(*value)?; None }

        NodeType::Utf16Array(values) => {
            buffer.write_u8(UTF16_ARRAY)?;
            let at = buffer.len();
            buffer.write_u32(0)?;
            for value in values { buffer.write_sized_string_u16(value)?; }
            Some(at)
        }
        NodeType::AsciiArray(values) => {
            buffer.write_u8(ASCII_ARRAY)?;
            let at = buffer.len();
            buffer.write_u32(0)?;
            for value in values { buffer.write_sized_string_u8(value)?; }
            Some(at)
        }

        other => {
            if !ESF::save_primitive(buffer, other)? {
                return Err(RLibError::EncodingESFUnsupportedNodeType(format!("{other:?}")));
            }
            None
        }
    };

    if let Some(at) = size_at {
        let end = (base + buffer.len() as u64) as u32;
        buffer[at..at + 4].copy_from_slice(&end.to_le_bytes());
    }

    Ok(())
}

/// Reads the metadata of a CEAB node, skipping its payload without decoding it.
pub(crate) fn read_node_meta<R: ReadBytes>(data: &mut R) -> Result<NodeMeta> {
    let start = data.stream_position()?;
    let next_byte = data.read_u8()?;

    if next_byte != RECORD && next_byte != RECORD_BLOCK {
        skip_leaf(data, next_byte)?;
        let end = data.stream_position()?;

        return Ok(NodeMeta {
            kind: reader::node_kind(next_byte, ESFSignature::CEAB),
            size: end - start,
            is_record: false,
            flags: RecordNodeFlags::empty(),
            version: 0,
            name_index: 0,
            groups: Vec::new(),
        });
    }

    let is_block = next_byte == RECORD_BLOCK;
    let name_index = data.read_u16()?;
    let version = data.read_u8()?;
    let final_block_offset = data.read_u32()? as u64;
    let group_count = if is_block { data.read_u32()? } else { 1 };

    let mut groups = Vec::with_capacity(group_count as usize);
    for _ in 0..group_count {
        let end = if is_block {
            data.read_u32()? as u64
        } else {
            final_block_offset
        };

        groups.push((data.stream_position()?, end));
        data.seek(SeekFrom::Start(end))?;
    }

    let end = data.stream_position()?;
    let flags = if is_block {
        RecordNodeFlags::IS_RECORD_NODE | RecordNodeFlags::HAS_NESTED_BLOCKS
    } else {
        RecordNodeFlags::IS_RECORD_NODE
    };

    Ok(NodeMeta {
        kind: "record",
        size: end - start,
        is_record: true,
        flags,
        version,
        name_index,
        groups,
    })
}

/// Skips a CEAB leaf node's payload without materializing it.
fn skip_leaf<R: ReadBytes>(data: &mut R, next_byte: u8) -> Result<()> {
    match next_byte {
        INVALID => Err(RLibError::DecodingESFUnsupportedDataType(next_byte)),

        UTF16 => {
            let length = data.read_u16()? as i64;
            data.seek(SeekFrom::Current(length * 2))?;
            Ok(())
        }
        ASCII => {
            let length = data.read_u16()? as i64;
            data.seek(SeekFrom::Current(length))?;
            Ok(())
        }

        UTF16_ARRAY | ASCII_ARRAY | UNKNOWN_21 | UNKNOWN_25 => {
            if next_byte == UNKNOWN_21 || next_byte == UNKNOWN_25 {
                data.read_u32()?;
            } else {
                let end_offset = data.read_u32()? as u64;
                data.seek(SeekFrom::Start(end_offset))?;
            }
            Ok(())
        }

        _ => {
            if ESF::is_primitive_array(next_byte) {
                let end_offset = data.read_u32()? as u64;
                data.seek(SeekFrom::Start(end_offset))?;
                return Ok(());
            }

            if ESF::skip_primitive(data, next_byte)? {
                return Ok(());
            }

            Err(RLibError::DecodingESFUnsupportedDataType(next_byte))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Seek};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn reads_abce_header_with_rpfm_primitives() {
        let mut reader = Cursor::new(vec![
            0xCE, 0xAB, 0x00, 0x00, 0x78, 0x56, 0x34, 0x12, 0xEF, 0xCD, 0xAB, 0x89,
        ]);

        let header = AbceCodec::default().read_header(&mut reader).unwrap();

        assert_eq!(header.id, ABCE_CODEC_ID);
        assert_eq!(header.unknown_1, 0x1234_5678);
        assert_eq!(header.edit_time, 0x89AB_CDEF);
        assert_eq!(reader.stream_position().unwrap(), 12);
    }

    #[test]
    fn writes_abce_header_with_current_unix_timestamp() {
        let before = AbceCodec::get_timestamp(SystemTime::now()).unwrap();
        let mut output = Vec::new();

        AbceCodec::default().write_header(&mut output).unwrap();

        let after = AbceCodec::get_timestamp(SystemTime::now()).unwrap();
        let mut reader = Cursor::new(output);
        let header = AbceCodec::default().read_header(&mut reader).unwrap();

        assert_eq!(header.id, ABCE_CODEC_ID);
        assert_eq!(header.unknown_1, 0);
        assert!((before..=after).contains(&header.edit_time));
    }

    #[test]
    fn converts_unix_timestamps() {
        let stamp = 1_764_961_050;
        let time = UNIX_EPOCH + Duration::from_secs(u64::from(stamp));

        assert_eq!(AbceCodec::get_time(stamp), time);
        assert_eq!(AbceCodec::get_timestamp(time).unwrap(), stamp);
    }

    #[test]
    fn rejects_timestamps_outside_the_header_range() {
        let after_u32 = UNIX_EPOCH + Duration::from_secs(u64::from(u32::MAX) + 1);
        assert!(AbceCodec::get_timestamp(after_u32).is_err());
        assert!(AbceCodec::get_timestamp(UNIX_EPOCH - Duration::from_secs(1)).is_err());
    }

    #[test]
    fn reads_header_using_configured_codec_id_without_rejecting_input_id() {
        let codec = AbceCodec::new(0x1234);
        let mut reader = Cursor::new(vec![1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);

        assert_eq!(codec.id(), 0x1234);
        assert_eq!(
            codec.read_header(&mut reader).unwrap(),
            AbceHeader {
                id: 1,
                unknown_1: 2,
                edit_time: 3
            }
        );
    }
}
