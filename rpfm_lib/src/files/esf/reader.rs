//! Module with [`ESFReader`], a lazy/on-demand reader for ESF files.
//!
//! [`ESF::decode`] decodes the whole node tree in one go. For big files (campaign saves,
//! startpos, ...) that means paying for every single node, even the ones you never look at.
//!
//! `ESFReader` instead only parses the file header and the shared string tables up front.
//! After that, nodes are reached on demand through [`NodeId`], and only the nodes you
//! actually ask for are ever decoded. Every record node knows its own byte size, so whole
//! subtrees can be skipped without reading them at all.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::str::FromStr;

use serde_derive::{Deserialize, Serialize};

use crate::binary::{ReadBytes, WriteBytes};
use crate::compression::Decompressible;
use crate::error::{RLibError, Result};

use super::*;

//---------------------------------------------------------------------------//
//                              Enum & Structs
//---------------------------------------------------------------------------//

/// Identifies a single node inside an [`ESFReader`].
///
/// `buffer` selects which buffer the node lives in (`0` is the file itself, `1+` are the
/// nested ESFs produced by [`ESFReader::decompress`]). `offset` is the byte offset of the
/// node inside that buffer.
///
/// Having a buffer id matters because a nested ESF has its own offsets, relative to its own
/// (decompressed) bytes. A bare offset would be ambiguous between buffers.
#[derive(PartialEq, Eq, Clone, Copy, Debug, Hash, Serialize, Deserialize)]
pub struct NodeId {
    buffer: u32,
    offset: u64,
}

/// A lightweight, header-only view of a node. No payload is decoded to build one of these.
#[derive(PartialEq, Clone, Debug, Serialize)]
pub struct NodeStub {

    /// Id of this node, to be passed back to the reader.
    pub id: NodeId,

    /// Type of the node, as a lowercase name (`"record"`, `"u32"`, `"u8_array"`, ...).
    pub kind: &'static str,

    /// Name of the node. Only records have one.
    pub name: Option<String>,

    /// Version of the node. Only records have one.
    pub version: Option<u8>,

    /// Raw record flags. Only records have them.
    pub flags: Option<u8>,

    /// Total size of the node in bytes, header included.
    pub size: u64,
}

/// Everything [`ESFReader`] needs to know about one buffer (the file itself, or a nested ESF).
struct EsfBuffer {
    data: Vec<u8>,
    signature: ESFSignature,
    unknown_1: u32,
    creation_date: u32,
    record_names: Vec<String>,
    strings_utf8: BTreeMap<u32, String>,
    strings_utf16: BTreeMap<u32, String>,
    root_offset: u64,
}

/// The metadata of a node, and the byte ranges of each of its children groups.
///
/// Building this never decodes a payload: records are skipped by their own block size, and
/// leaf payloads are skipped without being materialized.
pub(crate) struct NodeMeta {
    pub(crate) kind: &'static str,
    pub(crate) size: u64,
    pub(crate) is_record: bool,
    pub(crate) flags: RecordNodeFlags,
    pub(crate) version: u8,
    pub(crate) name_index: u16,
    pub(crate) groups: Vec<(u64, u64)>,
}

//---------------------------------------------------------------------------//
//                           Implementation of ESFReader
//---------------------------------------------------------------------------//

/// A lazy reader over an ESF file.
///
/// Create it with [`ESFReader::new`], then navigate and decode nodes on demand starting from
/// [`ESFReader::root_id`].
pub struct ESFReader {
    buffers: Vec<EsfBuffer>,
}

impl ESFReader {

    /// Creates a new reader over the provided ESF data.
    ///
    /// This only parses the file header and the shared string tables. The node tree itself is
    /// left untouched until you ask for a node.
    pub fn new(data: Vec<u8>) -> Result<Self> {
        Ok(Self {
            buffers: vec![EsfBuffer::parse(data)?],
        })
    }

    /// Returns the signature of the file.
    pub fn signature(&self) -> ESFSignature {
        self.buffers[0].signature
    }

    /// Returns the `unknown_1` value of the file.
    pub fn unknown_1(&self) -> u32 {
        self.buffers[0].unknown_1
    }

    /// Returns the creation date of the file.
    pub fn creation_date(&self) -> u32 {
        self.buffers[0].creation_date
    }

    /// Returns the id of the root node of the file.
    pub fn root_id(&self) -> NodeId {
        NodeId {
            buffer: 0,
            offset: self.buffers[0].root_offset,
        }
    }

    /// Returns the header info of a single node, without decoding anything else.
    pub fn node(&self, id: NodeId) -> Result<NodeStub> {
        let buffer = self.buffer(id.buffer)?;
        let meta = buffer.meta(id.offset)?;
        buffer.stub(id.buffer, id.offset, &meta)
    }

    /// Returns the children of a record node, grouped like they are stored in the file.
    ///
    /// Most records have a single group, so this is usually a one-element array. Records with
    /// the `HAS_NESTED_BLOCKS` flag have one entry per nested block.
    ///
    /// Only the children headers are read here: grandchildren are skipped, never decoded.
    pub fn children(&self, id: NodeId) -> Result<Vec<Vec<NodeStub>>> {
        let buffer = self.buffer(id.buffer)?;
        let meta = buffer.meta(id.offset)?;

        if !meta.is_record {
            return Ok(Vec::new());
        }

        let mut cursor = Cursor::new(buffer.data.as_slice());
        let mut groups = Vec::with_capacity(meta.groups.len());

        for (start, end) in meta.groups {
            cursor.seek(SeekFrom::Start(start))?;

            let mut stubs = Vec::new();
            while cursor.stream_position()? < end {
                let offset = cursor.stream_position()?;
                let child = read_meta(&mut cursor, buffer.signature, false)?;
                stubs.push(buffer.stub(id.buffer, offset, &child)?);
            }

            groups.push(stubs);
        }

        Ok(groups)
    }

    /// Decodes the value of a single node, on demand.
    ///
    /// This fails if the node is a record (a record has no single value), which also protects
    /// you from accidentally decoding a whole subtree with this call.
    pub fn value(&self, id: NodeId) -> Result<NodeType> {
        let buffer = self.buffer(id.buffer)?;
        let node = buffer.read_node(id.offset, id.offset == buffer.root_offset)?;

        if matches!(node, NodeType::Record(_)) {
            return Err(RLibError::DecodingESFNodeIsRecord(id.to_string()));
        }

        Ok(node)
    }

    /// Decodes the whole subtree starting at a node, on demand.
    ///
    /// This is the eager path, but scoped to just the subtree you ask for.
    pub fn materialize(&self, id: NodeId) -> Result<NodeType> {
        let buffer = self.buffer(id.buffer)?;
        buffer.read_node(id.offset, id.offset == buffer.root_offset)
    }

    /// Returns the raw bytes of a node, without decoding them at all.
    pub fn chunk(&self, id: NodeId) -> Result<Vec<u8>> {
        let buffer = self.buffer(id.buffer)?;
        let meta = buffer.meta(id.offset)?;
        buffer.bytes(id.offset, meta.size)
    }

    /// Resolves a `/`-separated path of record names into a node id.
    ///
    /// The path is relative to the root node, and every segment addresses a record by name.
    /// A segment may carry an index for the repeated-name case (`CHARACTER[2]`). Returns
    /// `None` if any segment does not resolve.
    ///
    /// Note this walks headers from the root on every call. Cache the returned id if you are
    /// going to reuse it.
    pub fn lookup(&self, path: &str) -> Result<Option<NodeId>> {
        let mut current = self.root_id();

        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            let (name, index) = parse_segment(segment);
            match self.find_child(current, name, index)? {
                Some(next) => current = next,
                None => return Ok(None),
            }
        }

        Ok(Some(current))
    }

    /// Decompresses the nested ESF hidden inside a `COMPRESSED_DATA` record.
    ///
    /// The nested ESF is registered as a new buffer, and the id of its root node is returned.
    /// All the other reader functions work on it as usual. The decompression only happens
    /// when you call this, never before.
    pub fn decompress(&mut self, id: NodeId) -> Result<NodeId> {
        let decompressed = {
            let buffer = self.buffer(id.buffer)?;
            let node = buffer.read_node(id.offset, id.offset == buffer.root_offset)?;
            compressed_payload(&node, id)?
        };

        let new_buffer = EsfBuffer::parse(decompressed)?;
        let root_offset = new_buffer.root_offset;
        self.buffers.push(new_buffer);

        Ok(NodeId {
            buffer: (self.buffers.len() - 1) as u32,
            offset: root_offset,
        })
    }

    /// Returns the buffer with the provided index, or an error if it does not exist.
    fn buffer(&self, index: u32) -> Result<&EsfBuffer> {
        self.buffers
            .get(index as usize)
            .ok_or_else(|| RLibError::DecodingESFInvalidNodeId(index.to_string()))
    }

    /// Finds the n-th child record with the provided name.
    fn find_child(&self, parent: NodeId, name: &str, index: usize) -> Result<Option<NodeId>> {
        let mut seen = 0usize;

        for group in self.children(parent)? {
            for stub in group {
                if stub.name.as_deref() == Some(name) {
                    if seen == index {
                        return Ok(Some(stub.id));
                    }

                    seen += 1;
                }
            }
        }

        Ok(None)
    }
}

//---------------------------------------------------------------------------//
//                           Implementation of EsfBuffer
//---------------------------------------------------------------------------//

impl EsfBuffer {

    /// Parses the header and the shared string tables of a buffer. The node tree is not touched.
    fn parse(data: Vec<u8>) -> Result<Self> {
        let mut cursor = Cursor::new(data);

        let sig_bytes = cursor.read_slice(4, false)?;
        let signature = ESFSignature::try_from(sig_bytes)?;

        let unknown_1 = cursor.read_u32()?;
        let creation_date = cursor.read_u32()?;
        let record_names_offset = cursor.read_u32()?;
        let root_offset = cursor.stream_position()?;

        // The string tables live after the nodes, and are referenced by the nodes, so they
        // have to be parsed before any node can be resolved.
        cursor.seek(SeekFrom::Start(record_names_offset as u64))?;

        let record_names = Self::read_record_names(&mut cursor, signature)?;
        let (strings_utf16, strings_utf8) = Self::read_string_tables(&mut cursor, signature)?;

        Ok(Self {
            data: cursor.into_inner(),
            signature,
            unknown_1,
            creation_date,
            record_names,
            strings_utf8,
            strings_utf16,
            root_offset,
        })
    }

    /// Reads the shared record-name table. CEAB and CFAB store their count as a signed 16-bit value.
    fn read_record_names<R: ReadBytes>(cursor: &mut R, signature: ESFSignature) -> Result<Vec<String>> {
        let count = if matches!(signature, ESFSignature::CEAB | ESFSignature::CFAB) {
            let count = cursor.read_i16()?;
            if count < 0 {
                return Err(RLibError::DecodingMismatchSizeError(0, count.unsigned_abs() as usize));
            }

            count as usize
        } else {
            cursor.read_u16()? as usize
        };

        let mut record_names = Vec::with_capacity(count);
        for _ in 0..count {
            record_names.push(cursor.read_sized_string_u8()?);
        }

        Ok(record_names)
    }

    /// Reads the shared string tables. CEAB stores its strings inline in the nodes, so it has none.
    fn read_string_tables<R: ReadBytes>(
        cursor: &mut R,
        signature: ESFSignature,
    ) -> Result<(BTreeMap<u32, String>, BTreeMap<u32, String>)> {
        if signature == ESFSignature::CEAB {
            return Ok((BTreeMap::new(), BTreeMap::new()));
        }

        // CBAB is the only format that prefixes its strings with a 32-bit size.
        let count_utf16 = cursor.read_u32()?;
        let mut strings_utf16 = BTreeMap::new();
        for _ in 0..count_utf16 {
            let name = if signature == ESFSignature::CBAB {
                cursor.read_sized_string_u16_u32()?
            } else {
                cursor.read_sized_string_u16()?
            };
            let index = cursor.read_u32()?;
            strings_utf16.insert(index, name);
        }

        let count_utf8 = cursor.read_u32()?;
        let mut strings_utf8 = BTreeMap::new();
        for _ in 0..count_utf8 {
            let name = if signature == ESFSignature::CBAB {
                cursor.read_sized_string_u8_u32()?
            } else {
                cursor.read_sized_string_u8()?
            };
            let index = cursor.read_u32()?;
            strings_utf8.insert(index, name);
        }

        Ok((strings_utf16, strings_utf8))
    }

    /// Reads the metadata of the node at the provided offset.
    fn meta(&self, offset: u64) -> Result<NodeMeta> {
        let mut cursor = Cursor::new(self.data.as_slice());
        cursor.seek(SeekFrom::Start(offset))?;
        read_meta(&mut cursor, self.signature, offset == self.root_offset)
    }

    /// Fully decodes the node at the provided offset.
    fn read_node(&self, offset: u64, is_root: bool) -> Result<NodeType> {
        let mut cursor = Cursor::new(self.data.as_slice());
        cursor.seek(SeekFrom::Start(offset))?;

        // Each format walks its nodes its own way, so the walker is picked per format.
        match self.signature {
            ESFSignature::CAAB => super::caab::read_node(&mut cursor, is_root, &self.record_names, &self.strings_utf8, &self.strings_utf16),
            ESFSignature::CBAB => super::cbab::read_node(&mut cursor, is_root, &self.record_names, &self.strings_utf8, &self.strings_utf16),
            ESFSignature::CEAB => super::ceab::read_node(&mut cursor, &self.record_names),
            ESFSignature::CFAB => super::cfab::read_node(&mut cursor, &self.record_names, &self.strings_utf8, &self.strings_utf16),
        }
    }

    /// Returns a raw slice of the buffer.
    fn bytes(&self, offset: u64, size: u64) -> Result<Vec<u8>> {
        let mut cursor = Cursor::new(self.data.as_slice());
        cursor.seek(SeekFrom::Start(offset))?;
        cursor.read_slice(size as usize, false)
    }

    /// Builds a stub from a node metadata.
    fn stub(&self, buffer: u32, offset: u64, meta: &NodeMeta) -> Result<NodeStub> {
        let name = if meta.is_record {
            Some(
                self.record_names
                    .get(meta.name_index as usize)
                    .ok_or(RLibError::DecodingESFRecordNameNotFound(meta.name_index))?
                    .clone(),
            )
        } else {
            None
        };

        Ok(NodeStub {
            id: NodeId { buffer, offset },
            kind: meta.kind,
            name,
            version: if meta.is_record { Some(meta.version) } else { None },
            flags: if meta.is_record { Some(meta.flags.bits()) } else { None },
            size: meta.size,
        })
    }
}

//---------------------------------------------------------------------------//
//                              Implementation of NodeId
//---------------------------------------------------------------------------//

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.buffer, self.offset)
    }
}

impl FromStr for NodeId {
    type Err = RLibError;

    fn from_str(value: &str) -> Result<Self> {
        let (buffer, offset) = value
            .split_once(':')
            .ok_or_else(|| RLibError::DecodingESFInvalidNodeId(value.to_owned()))?;

        let buffer = buffer
            .parse::<u32>()
            .map_err(|_| RLibError::DecodingESFInvalidNodeId(value.to_owned()))?;

        let offset = offset
            .parse::<u64>()
            .map_err(|_| RLibError::DecodingESFInvalidNodeId(value.to_owned()))?;

        Ok(Self { buffer, offset })
    }
}

//---------------------------------------------------------------------------//
//                                  Functions
//---------------------------------------------------------------------------//

/// Reads a node's metadata using the rules of the format the buffer is in.
///
/// Each format measures records its own way, so the walker is picked here rather than being
/// folded into one implementation with format checks inside it.
fn read_meta<R: ReadBytes>(data: &mut R, signature: ESFSignature, is_root: bool) -> Result<NodeMeta> {
    match signature {
        ESFSignature::CEAB => super::ceab::read_node_meta(data),
        ESFSignature::CFAB => super::cfab::read_node_meta(data),
        _ => read_node_meta(data, is_root, signature),
    }
}

/// Reads a node's metadata, skipping its payload without decoding it.
///
/// Records are skipped using their own block size, so their children are never visited.
/// Leaf payloads are skipped without being materialized.
fn read_node_meta<R: ReadBytes>(data: &mut R, is_root: bool, signature: ESFSignature) -> Result<NodeMeta> {
    let start = data.stream_position()?;
    let next_byte = data.read_u8()?;

    if next_byte & RecordNodeFlags::IS_RECORD_NODE.bits() == RecordNodeFlags::IS_RECORD_NODE.bits() {
        let flags = RecordNodeFlags::from_bits_truncate(next_byte);
        let has_non_optimized_info =
            flags.contains(RecordNodeFlags::HAS_NON_OPTIMIZED_INFO) || is_root;

        let (name_index, version) = if has_non_optimized_info {
            (data.read_u16()?, data.read_u8()?)
        } else {
            let version = (next_byte & 0x1E) >> 1;
            let name_index = (((next_byte & 1) as u16) << 8) + data.read_u8()? as u16;
            (name_index, version)
        };

        let block_size = data.read_cauleb128()?;
        let has_nested_blocks = flags.contains(RecordNodeFlags::HAS_NESTED_BLOCKS);
        let group_count = if has_nested_blocks {
            data.read_cauleb128()?
        } else {
            1
        };
        let final_block_offset = data.stream_position()? + block_size as u64;

        // Collect the byte range of each group, and skip over its contents in one go.
        let mut groups = Vec::with_capacity(group_count as usize);
        for _ in 0..group_count {
            let end = if has_nested_blocks {
                let entry_size = data.read_cauleb128()?;
                data.stream_position()? + entry_size as u64
            } else {
                final_block_offset
            };

            groups.push((data.stream_position()?, end));
            data.seek(SeekFrom::Start(end))?;
        }

        let end = data.stream_position()?;

        Ok(NodeMeta {
            kind: "record",
            size: end - start,
            is_record: true,
            flags,
            version,
            name_index,
            groups,
        })
    } else {
        skip_leaf(data, next_byte, signature)?;
        let end = data.stream_position()?;

        Ok(NodeMeta {
            kind: node_kind(next_byte, signature),
            size: end - start,
            is_record: false,
            flags: RecordNodeFlags::empty(),
            version: 0,
            name_index: 0,
            groups: Vec::new(),
        })
    }
}

/// Skips a leaf node's payload without materializing it.
fn skip_leaf<R: ReadBytes>(data: &mut R, next_byte: u8, signature: ESFSignature) -> Result<()> {
    match next_byte {

        // Invalid node. This is always an error.
        INVALID => return Err(RLibError::DecodingESFUnsupportedDataType(next_byte)),

        //------------------------------------------------//
        // Primitive nodes.
        //------------------------------------------------//
        BOOL => { data.read_u8()?; }
        I8 | U8 => { data.read_u8()?; }
        I16 | U16 | ANGLE => { data.read_u16()?; }
        I32 | U32 | F32 => { data.read_u32()?; }
        I64 | U64 | F64 => { data.read_u64()?; }

        // CEAB stores its strings inline, so their size is not fixed. Every other format
        // stores a fixed-size reference into the shared string tables.
        UTF16 => {
            if signature == ESFSignature::CEAB {
                let length = data.read_u16()? as i64;
                data.seek(SeekFrom::Current(length * 2))?;
            } else {
                data.read_u32()?;
            }
        }
        ASCII => {
            if signature == ESFSignature::CEAB {
                let length = data.read_u16()? as i64;
                data.seek(SeekFrom::Current(length))?;
            } else {
                data.read_u32()?;
            }
        }

        //------------------------------------------------//
        // Complex nodes.
        //------------------------------------------------//
        COORD_2D => { data.read_slice(8, false)?; }
        COORD_3D => { data.read_slice(12, false)?; }

        //------------------------------------------------//
        // Unknown nodes.
        //------------------------------------------------//
        UNKNOWN_21 | UNKNOWN_25 => { data.read_u32()?; }
        UNKNOWN_23 => { data.read_u8()?; }
        UNKNOWN_24 => { data.read_u16()?; }
        UNKNOWN_26 => {
            let first_byte = data.read_u8()?;
            if first_byte % 8 == 0 && first_byte != 0 {
                data.read_slice(first_byte as usize, false)?;
            } else {
                data.read_slice(7, false)?;
            }

            // Peek at the next byte. If it's not the 0x9C terminator, it belongs to whatever
            // comes next, so put it back.
            if let Ok(last_byte) = data.read_u8() {
                if last_byte != 0x9C {
                    data.seek(SeekFrom::Current(-1))?;
                }
            }
        }

        //------------------------------------------------//
        // Optimized primitive nodes (no payload).
        //------------------------------------------------//
        BOOL_TRUE | BOOL_FALSE | U32_ZERO | U32_ONE | I32_ZERO | F32_ZERO => {}
        U32_BYTE | I32_BYTE => { data.read_u8()?; }
        U32_16BIT | I32_16BIT => { data.read_u16()?; }
        U32_24BIT | I32_24BIT => { data.read_u24()?; }

        //------------------------------------------------//
        // Arrays of primitives, optimized or not.
        //
        // Every one of them stores its payload length as a cauleb128 before the payload, so
        // they can all be skipped the same way.
        //------------------------------------------------//
        BOOL_ARRAY | I8_ARRAY | I16_ARRAY | I32_ARRAY | I64_ARRAY | U8_ARRAY | U16_ARRAY
        | U32_ARRAY | U64_ARRAY | F32_ARRAY | F64_ARRAY | COORD_2D_ARRAY | COORD_3D_ARRAY
        | UTF16_ARRAY | ASCII_ARRAY | ANGLE_ARRAY
        | BOOL_TRUE_ARRAY | BOOL_FALSE_ARRAY | U32_ZERO_ARRAY | U32_ONE_ARRAY
        | U32_BYTE_ARRAY | U32_16BIT_ARRAY | U32_24BIT_ARRAY
        | I32_ZERO_ARRAY | I32_BYTE_ARRAY | I32_16BIT_ARRAY | I32_24BIT_ARRAY
        | F32_ZERO_ARRAY => {
            let size = data.read_cauleb128()?;
            data.seek(SeekFrom::Current(size as i64))?;
        }

        // Anything else is not yet supported.
        _ => return Err(RLibError::DecodingESFUnsupportedDataType(next_byte)),
    }

    Ok(())
}

/// Returns the friendly name of a node type, from its marker byte.
pub(crate) fn node_kind(next_byte: u8, signature: ESFSignature) -> &'static str {
    match next_byte {
        BOOL | BOOL_TRUE | BOOL_FALSE => "bool",
        I8 => "i8",
        I16 => "i16",
        I32 | I32_ZERO | I32_BYTE | I32_16BIT | I32_24BIT => "i32",
        I64 => "i64",
        U8 => "u8",
        U16 => "u16",
        U32 | U32_ZERO | U32_ONE | U32_BYTE | U32_16BIT | U32_24BIT => "u32",
        U64 => "u64",
        F32 | F32_ZERO => "f32",
        F64 => "f64",
        COORD_2D => "coord2d",
        COORD_3D => "coord3d",
        UTF16 => "utf16",
        ASCII => "ascii",
        ANGLE => "angle",
        UNKNOWN_21 if signature == ESFSignature::CFAB => "ascii_w21",
        UNKNOWN_21 => "unknown21",
        UNKNOWN_23 => "unknown23",
        UNKNOWN_24 => "unknown24",
        UNKNOWN_25 if signature == ESFSignature::CFAB => "ascii_w25",
        UNKNOWN_25 => "unknown25",
        UNKNOWN_26 => "unknown26",
        BOOL_ARRAY | BOOL_TRUE_ARRAY | BOOL_FALSE_ARRAY => "bool_array",
        I8_ARRAY => "i8_array",
        I16_ARRAY => "i16_array",
        I32_ARRAY | I32_ZERO_ARRAY | I32_BYTE_ARRAY | I32_16BIT_ARRAY | I32_24BIT_ARRAY => "i32_array",
        I64_ARRAY => "i64_array",
        U8_ARRAY => "u8_array",
        U16_ARRAY => "u16_array",
        U32_ARRAY | U32_ZERO_ARRAY | U32_ONE_ARRAY | U32_BYTE_ARRAY | U32_16BIT_ARRAY | U32_24BIT_ARRAY => "u32_array",
        U64_ARRAY => "u64_array",
        F32_ARRAY | F32_ZERO_ARRAY => "f32_array",
        F64_ARRAY => "f64_array",
        COORD_2D_ARRAY => "coord2d_array",
        COORD_3D_ARRAY => "coord3d_array",
        UTF16_ARRAY => "utf16_array",
        ASCII_ARRAY => "ascii_array",
        ANGLE_ARRAY => "angle_array",
        _ => "unknown",
    }
}

/// Splits a path segment into a name and an optional index (`CHARACTER[2]`).
fn parse_segment(segment: &str) -> (&str, usize) {
    if let Some(open) = segment.rfind('[') {
        if segment.ends_with(']') {
            if let Ok(index) = segment[open + 1..segment.len() - 1].parse::<usize>() {
                return (&segment[..open], index);
            }
        }
    }

    (segment, 0)
}

/// Extracts and decompresses the nested ESF payload of a `COMPRESSED_DATA` node.
fn compressed_payload(node: &NodeType, id: NodeId) -> Result<Vec<u8>> {
    let not_compressed = || RLibError::DecodingESFNotCompressedData(id.to_string());

    let NodeType::Record(cnode) = node else {
        return Err(not_compressed());
    };

    if cnode.name != COMPRESSED_DATA_TAG {
        return Err(not_compressed());
    }

    let payload = match cnode.children().first().and_then(|group| group.first()) {
        Some(NodeType::U8Array(payload)) => payload,
        _ => return Err(not_compressed()),
    };

    let hnode = match cnode.children().first().and_then(|group| group.get(1)) {
        Some(NodeType::Record(hnode)) if hnode.name == COMPRESSED_DATA_INFO_TAG => hnode,
        _ => return Err(not_compressed()),
    };

    let len = match hnode.children().first().and_then(|group| group.first()) {
        Some(NodeType::U32(len)) => *len.value(),
        _ => return Err(not_compressed()),
    };

    let magic = match hnode.children().first().and_then(|group| group.get(1)) {
        Some(NodeType::U8Array(magic)) => magic,
        _ => return Err(not_compressed()),
    };

    let mut data = Vec::new();
    data.write_u32(len)?;
    data.write_all(magic)?;
    data.write_all(payload)?;

    data.as_slice().decompress()
}
