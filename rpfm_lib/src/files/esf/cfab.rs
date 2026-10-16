use std::collections::BTreeMap;
use std::io::{SeekFrom, Write};

use crate::binary::{ReadBytes, WriteBytes};
use crate::error::{RLibError, Result};

use super::ceab::{AbceCodec, AbceHeader};
use super::reader::NodeMeta;
use super::*;

/// Little-endian codec ID used by CFAB files.
pub const ABCF_CODEC_ID: u32 = 0xABCF;

/// ABCF header codec, using the same header layout as [`AbceCodec`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbcfCodec {
    header_codec: AbceCodec,
}

impl Default for AbcfCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl AbcfCodec {
    /// Creates the ABCF codec with its standard ID.
    pub const fn new() -> Self {
        Self {
            header_codec: AbceCodec::new(ABCF_CODEC_ID),
        }
    }

    /// Returns this codec's numeric ID.
    pub const fn id(&self) -> u32 {
        self.header_codec.id()
    }

    /// Reads the ID, unknown value, and edit timestamp from an ABCF header.
    pub fn read_header<R: ReadBytes>(&self, reader: &mut R) -> Result<AbceHeader> {
        self.header_codec.read_header(reader)
    }

    /// Writes the ID, zeroed unknown value, and current edit timestamp.
    pub fn write_header<W: WriteBytes>(&self, writer: &mut W) -> Result<()> {
        self.header_codec.write_header(writer)
    }

    pub(crate) fn read_utf16_string_list<R: ReadBytes>(
        &self,
        reader: &mut R,
    ) -> Result<BTreeMap<u32, String>> {
        Self::read_string_list(reader, ReadBytes::read_sized_string_u16)
    }

    pub(crate) fn read_ascii_string_list<R: ReadBytes>(
        &self,
        reader: &mut R,
    ) -> Result<BTreeMap<u32, String>> {
        Self::read_string_list(reader, ReadBytes::read_sized_string_u8)
    }

    pub(crate) fn write_utf16_string_list<W: WriteBytes>(
        &self,
        writer: &mut W,
        strings: &[String],
    ) -> Result<()> {
        Self::write_string_list(writer, strings, WriteBytes::write_sized_string_u16)
    }

    pub(crate) fn write_ascii_string_list<W: WriteBytes>(
        &self,
        writer: &mut W,
        strings: &[String],
    ) -> Result<()> {
        Self::write_string_list(writer, strings, WriteBytes::write_sized_string_u8)
    }

    pub(crate) fn read_string_reference(
        &self,
        index: u32,
        strings: &BTreeMap<u32, String>,
    ) -> Result<String> {
        strings
            .get(&index)
            .cloned()
            .ok_or(RLibError::DecodingESFStringNotFound(index))
    }

    pub(crate) fn write_string_reference(&self, value: &str, strings: &[String]) -> Result<u32> {
        strings
            .iter()
            .position(|string| string == value)
            .map(|index| index as u32)
            .ok_or_else(|| RLibError::EncodingESFStringNotFound(value.to_owned()))
    }

    fn read_string_list<R: ReadBytes>(
        reader: &mut R,
        read_string: fn(&mut R) -> Result<String>,
    ) -> Result<BTreeMap<u32, String>> {
        let count = reader.read_u32()?;
        let mut strings = BTreeMap::new();
        for _ in 0..count {
            let string = read_string(reader)?;
            let index = reader.read_u32()?;
            strings.insert(index, string);
        }

        Ok(strings)
    }

    fn write_string_list<W: WriteBytes>(
        writer: &mut W,
        strings: &[String],
        write_string: fn(&mut W, &str) -> Result<()>,
    ) -> Result<()> {
        writer.write_u32(strings.len() as u32)?;
        for (index, string) in strings.iter().enumerate() {
            write_string(writer, string)?;
            writer.write_u32(index as u32)?;
        }

        Ok(())
    }
}

impl ESF {
    pub(crate) fn read_cfab<R: ReadBytes>(&mut self, data: &mut R) -> Result<()> {
        self.unknown_1 = data.read_u32()?;
        self.creation_date = data.read_u32()?;
        let record_names_offset = data.read_u32()?;
        let nodes_offset = data.stream_position()?;

        data.seek(SeekFrom::Start(record_names_offset as u64))?;

        // Like CEAB, the record-name count is a signed 16-bit value.
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

        let codec = AbcfCodec::default();
        let strings_utf16 = codec.read_utf16_string_list(data)?;
        let strings_utf8 = codec.read_ascii_string_list(data)?;

        // The tables are the last thing in a well-formed file, but real files can carry zero
        // padding after them. Anything else means the tables parsed to the wrong place.
        let current_pos = data.stream_position()?;
        let data_len = data.len()?;
        if current_pos != data_len && !ESF::only_padding_remains(data)? {
            return Err(RLibError::DecodingMismatchSizeError(
                data_len as usize,
                current_pos as usize,
            ));
        }

        data.seek(SeekFrom::Start(nodes_offset))?;
        self.root_node = read_node(data, &record_names, &strings_utf8, &strings_utf16)?;

        let current_pos = data.stream_position()?;
        if current_pos != record_names_offset as u64 {
            return Err(RLibError::DecodingMismatchSizeError(
                record_names_offset as usize,
                current_pos as usize,
            ));
        }

        Ok(())
    }

    pub(crate) fn save_cfab<W: WriteBytes>(
        &mut self,
        buffer: &mut W,
        _extra_data: &Option<EncodeableExtraData>,
    ) -> Result<()> {
        buffer.write_u32(self.unknown_1)?;
        buffer.write_u32(self.creation_date)?;

        let mut record_names = Vec::new();
        let mut strings_utf8 = Vec::new();
        let mut strings_utf16 = Vec::new();
        Self::read_string_from_node(
            &self.root_node,
            &mut record_names,
            &mut strings_utf8,
            &mut strings_utf16,
        );

        let mut nodes_data = Vec::new();
        save_node(
            &mut nodes_data,
            16,
            &self.root_node,
            &record_names,
            &strings_utf8,
            &strings_utf16,
        )?;

        let mut strings_data = Vec::new();
        strings_data.write_i16(record_names.len() as i16)?;
        for name in &record_names {
            strings_data.write_sized_string_u8(name)?;
        }

        let codec = AbcfCodec::default();
        codec.write_utf16_string_list(&mut strings_data, &strings_utf16)?;
        codec.write_ascii_string_list(&mut strings_data, &strings_utf8)?;

        buffer.write_u32((12 + nodes_data.len() + 4) as u32)?;
        buffer.write_all(&nodes_data)?;
        buffer.write_all(&strings_data)?;

        Ok(())
    }
}

//---------------------------------------------------------------------------//
//                          CFAB node decoding/encoding
//---------------------------------------------------------------------------//
//
// CFAB uses CEAB's node layout: 32-bit sizes and counts that are absolute end offsets,
// record info that is always a `u16` name index plus a `u8` version, and the `0x81` marker
// for a record holding several entries.
//
// The difference is strings. CFAB keeps them in two tables after the record names, and a node
// stores a 32-bit reference into those tables (string arrays included). The reference is the
// value stored next to each string, so lookups go through the maps read by `read_cfab`.
//
// `0x21` and `0x25` are not unknown values in CFAB: they are ASCII strings like any other.

/// Marker of a plain record.
const RECORD: u8 = 0x80;

/// Marker of a record holding several entries, each with its own end offset.
const RECORD_BLOCK: u8 = 0x81;

/// Reads a CFAB node, and everything below it.
pub(crate) fn read_node<R: ReadBytes>(
    data: &mut R,
    record_names: &[String],
    strings_utf8: &BTreeMap<u32, String>,
    strings_utf16: &BTreeMap<u32, String>,
) -> Result<NodeType> {
    let next_byte = data.read_u8()?;

    if next_byte != RECORD && next_byte != RECORD_BLOCK {
        return read_leaf(data, next_byte, strings_utf8, strings_utf16);
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
            nodes.push(read_node(data, record_names, strings_utf8, strings_utf16)?);
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

/// Reads a CFAB leaf node. Strings are references into the file's string tables.
fn read_leaf<R: ReadBytes>(
    data: &mut R,
    next_byte: u8,
    strings_utf8: &BTreeMap<u32, String>,
    strings_utf16: &BTreeMap<u32, String>,
) -> Result<NodeType> {
    let codec = AbcfCodec::default();

    match next_byte {
        INVALID => Err(RLibError::DecodingESFUnsupportedDataType(next_byte)),

        UTF16 => {
            let index = data.read_u32()?;
            Ok(NodeType::Utf16(codec.read_string_reference(index, strings_utf16)?))
        }
        ASCII => {
            let index = data.read_u32()?;
            Ok(NodeType::Ascii(codec.read_string_reference(index, strings_utf8)?))
        }

        UNKNOWN_21 => {
            let index = data.read_u32()?;
            Ok(NodeType::AsciiW21(codec.read_string_reference(index, strings_utf8)?))
        }
        UNKNOWN_25 => {
            let index = data.read_u32()?;
            Ok(NodeType::AsciiW25(codec.read_string_reference(index, strings_utf8)?))
        }

        UTF16_ARRAY => {
            let end_offset = data.read_u32()? as u64;
            let mut values = Vec::new();
            while data.stream_position()? < end_offset {
                let index = data.read_u32()?;
                values.push(codec.read_string_reference(index, strings_utf16)?);
            }
            Ok(NodeType::Utf16Array(values))
        }
        ASCII_ARRAY => {
            let end_offset = data.read_u32()? as u64;
            let mut values = Vec::new();
            while data.stream_position()? < end_offset {
                let index = data.read_u32()?;
                values.push(codec.read_string_reference(index, strings_utf8)?);
            }
            Ok(NodeType::AsciiArray(values))
        }

        _ => {
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

/// Writes a CFAB node, and everything below it, into `buffer`.
///
/// `base` is the absolute offset `buffer[0]` will end up at, because CFAB writes sizes as
/// absolute offsets.
pub(crate) fn save_node(
    buffer: &mut Vec<u8>,
    base: u64,
    node: &NodeType,
    record_names: &[String],
    strings_utf8: &[String],
    strings_utf16: &[String],
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
                save_node(buffer, base, child, record_names, strings_utf8, strings_utf16)?;
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

    let codec = AbcfCodec::default();

    let size_at = match node {
        NodeType::Utf16(value) => {
            buffer.write_u8(UTF16)?;
            buffer.write_u32(codec.write_string_reference(value, strings_utf16)?)?;
            None
        }
        NodeType::Ascii(value) => {
            buffer.write_u8(ASCII)?;
            buffer.write_u32(codec.write_string_reference(value, strings_utf8)?)?;
            None
        }
        NodeType::AsciiW21(value) => {
            buffer.write_u8(UNKNOWN_21)?;
            buffer.write_u32(codec.write_string_reference(value, strings_utf8)?)?;
            None
        }
        NodeType::AsciiW25(value) => {
            buffer.write_u8(UNKNOWN_25)?;
            buffer.write_u32(codec.write_string_reference(value, strings_utf8)?)?;
            None
        }

        NodeType::Utf16Array(values) => {
            buffer.write_u8(UTF16_ARRAY)?;
            let at = buffer.len();
            buffer.write_u32(0)?;
            for value in values {
                buffer.write_u32(codec.write_string_reference(value, strings_utf16)?)?;
            }
            Some(at)
        }
        NodeType::AsciiArray(values) => {
            buffer.write_u8(ASCII_ARRAY)?;
            let at = buffer.len();
            buffer.write_u32(0)?;
            for value in values {
                buffer.write_u32(codec.write_string_reference(value, strings_utf8)?)?;
            }
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

/// Reads the metadata of a CFAB node, skipping its payload without decoding it.
pub(crate) fn read_node_meta<R: ReadBytes>(data: &mut R) -> Result<NodeMeta> {
    let start = data.stream_position()?;
    let next_byte = data.read_u8()?;

    if next_byte != RECORD && next_byte != RECORD_BLOCK {
        skip_leaf(data, next_byte)?;
        let end = data.stream_position()?;

        return Ok(NodeMeta {
            kind: super::reader::node_kind(next_byte, ESFSignature::CFAB),
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

/// Skips a CFAB leaf node's payload without materializing it.
fn skip_leaf<R: ReadBytes>(data: &mut R, next_byte: u8) -> Result<()> {
    match next_byte {
        INVALID => Err(RLibError::DecodingESFUnsupportedDataType(next_byte)),

        // Strings, and the two string-ish markers, are fixed-size references.
        UTF16 | ASCII | UNKNOWN_21 | UNKNOWN_25 => {
            data.read_u32()?;
            Ok(())
        }

        UTF16_ARRAY | ASCII_ARRAY => {
            let end_offset = data.read_u32()? as u64;
            data.seek(SeekFrom::Start(end_offset))?;
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
    use std::io::Cursor;

    use super::*;

    #[test]
    fn uses_abcf_codec_id_and_abce_header_layout() {
        let codec = AbcfCodec::default();
        let mut bytes = Vec::new();

        assert_eq!(codec.id(), ABCF_CODEC_ID);
        codec.write_header(&mut bytes).unwrap();

        let header = codec.read_header(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(header.id, ABCF_CODEC_ID);
        assert_eq!(header.unknown_1, 0);
    }
}
