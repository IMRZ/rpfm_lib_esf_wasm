//---------------------------------------------------------------------------//
// Copyright (c) 2017-2024 Ismael Gutiérrez González. All rights reserved.
//
// This file is part of the Rusted PackFile Manager (RPFM) project,
// which can be found here: https://github.com/Frodo45127/rpfm.
//
// This file is licensed under the MIT license, which can be found here:
// https://github.com/Frodo45127/rpfm/blob/master/LICENSE.
//---------------------------------------------------------------------------//

//! Module with the logic and documentation for the CBAB ESF files.

use std::collections::BTreeMap;
use std::io::{Cursor, SeekFrom, Write};

use crate::binary::{ReadBytes, WriteBytes};
use crate::compression::{Compressible, CompressionFormat, Decompressible};
use crate::error::{RLibError, Result};

use super::*;

//---------------------------------------------------------------------------//
//                           Implementation of ESF
//---------------------------------------------------------------------------//

/// Implementation of `ESF`. Section of functions specific for the CBAB format.
impl ESF {

    /// This function creates a `ESF` of type CBAB from a `Vec<u8>`.
    pub(crate) fn read_cbab<R: ReadBytes>(&mut self, data: &mut R) -> Result<()> {

        // Note: this assumes the caller has already read the first 4 bytes of the data.
        self.unknown_1 = data.read_u32()?;
        self.creation_date = data.read_u32()?;
        let record_names_offset = data.read_u32()?;
        let nodes_offset = data.stream_position()?;

        // We need this data decoded first, because some nodes reference to it, and we can use that to populate the nodes.
        data.seek(SeekFrom::Start(record_names_offset as u64))?;

        // Get the name list for the record/record block entries.
        let record_names_count = data.read_u16()?;
        let mut record_names = vec![];
        for _ in 0..record_names_count {
            record_names.push(data.read_sized_string_u8()?);
        }

        // Get the UTF-16 Strings for all the subnodes.
        let strings_count_utf16 = data.read_u32()?;
        let mut strings_utf16 = BTreeMap::new();
        for _ in 0..strings_count_utf16 {
            let name = data.read_sized_string_u16_u32()?;
            let index = data.read_u32()?;
            strings_utf16.insert(index, name);
        }

        // Get the UTF-8 Strings for all the subnodes.
        let strings_count_utf8 = data.read_u32()?;
        let mut strings_utf8 = BTreeMap::new();
        for _ in 0..strings_count_utf8 {
            let name = data.read_sized_string_u8_u32()?;
            let index = data.read_u32()?;
            strings_utf8.insert(index, name);
        }

        // The string tables are the last thing in a well-formed file, but real files can carry
        // zero padding after them. Anything else means the tables parsed to the wrong place.
        let curr_pos = data.stream_position()?;
        let data_len = data.len()?;
        if curr_pos != data_len && !ESF::only_padding_remains(data)? {
            return Err(RLibError::DecodingMismatchSizeError(data_len as usize, curr_pos as usize));
        }

        // Restore the index before continuing.
        data.seek(SeekFrom::Start(nodes_offset))?;

        // This file is a big tree hanging from the root node, so just decode everything recursively.
        self.root_node = read_node(data, true, &record_names, &strings_utf8, &strings_utf16)?;

        // If we're not at the exact end of the nodes, something failed.
        let curr_pos = data.stream_position()?;
        if curr_pos != record_names_offset as u64 {
            return Err(RLibError::DecodingMismatchSizeError(record_names_offset as usize, curr_pos as usize));
        }

        // Once we're done with the nodes, we need to check if the last children of the root node contains a compressed record.
        // If so, that record will contain an entire ESF which we need to decode, and then replace ours with that one.
        //
        // The reason for this is, I guess, optimization. Some ESF files, specially the startpos ones, may have specific nodes that are enormous.
        // By keeping a compressed copy of the startpos, the game can read all the other nodes without loading the big ones to memory. But it's just a guess.
        if let NodeType::Record(ref mut node) = self.root_node {
            if let Some(child) = node.children_mut().get_mut(0) {
                if let Some(NodeType::Record(cnode)) = child.last_mut() {
                    if cnode.name == COMPRESSED_DATA_TAG {
                        let mut dec_data = vec![];
                        if let Some(NodeType::U8Array(data)) = cnode.children()[0].first() {
                            if let Some(NodeType::Record(hnode)) = cnode.children()[0].get(1) {
                                if hnode.name == COMPRESSED_DATA_INFO_TAG {
                                    if let Some(NodeType::U32(len)) = hnode.children()[0].first() {
                                        if let Some(NodeType::U8Array(magic_number)) = hnode.children()[0].get(1) {

                                            let mut mdata = vec![];
                                            mdata.write_u32(*len.value())?;
                                            mdata.write_all(magic_number)?;
                                            mdata.write_all(data)?;

                                            dec_data = mdata.as_slice().decompress()?;

                                            //let path_1 = "../test_files/test_decode_esf_caab.esf_decompressed";
                                            //let mut writer = std::io::BufWriter::new(std::fs::File::create(path_1).unwrap());
                                            //writer.write_all(&dec_data).unwrap();
                                        }
                                    }
                                }
                            }
                        }

                        if !dec_data.is_empty() {
                            let mut dec_datac = Cursor::new(dec_data.clone());
                            let new_esf = ESF::decode(&mut dec_datac, &None)?;
                            *self = new_esf;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// This function takes a `ESF` of type CBAB and encodes it to `Vec<u8>`.
    pub(crate) fn save_cbab<W: WriteBytes>(&mut self, buffer: &mut W, extra_data: &Option<EncodeableExtraData>) -> Result<()> {
        let mut extra_data = extra_data.clone().unwrap_or_default();
        let disable_compression = extra_data.disable_compression;

        let backup = self.clone();
        let mut revert_compression = false;
        let mut index = None;

        // If we have a known compressed node, encode the esf, compress it, then replace said node with the compressed esf.
        // Note that the operation alters the self, so we need to restore it at the end of this function in order to keep it usable.
        if !disable_compression {
            if let NodeType::Record(ref root_node) = self.root_node {
                for (i1, parent) in root_node.children().iter().enumerate() {
                    for (i2, child) in parent.iter().enumerate() {
                        if let NodeType::Record(ref cnode) = child {
                            if COMPRESSED_TAGS.contains(&&*cnode.name)  {
                                index = Some((i1, i2));
                                break;
                            }
                        }
                    }
                }
            }

            if let Some((i1, i2)) = index {
                let mut ncdata = vec![];

                extra_data.disable_compression = true;
                self.encode(&mut ncdata, &Some(extra_data))?;
                revert_compression = true;

                let mut fdata = ncdata.compress(CompressionFormat::Lzma1)?;
                let cdata = fdata.split_off(9);
                let mut hdata = Cursor::new(fdata);

                let hnode = RecordNode {
                    record_flags: RecordNodeFlags::IS_RECORD_NODE,
                    version: 0,
                    name: COMPRESSED_DATA_INFO_TAG.to_owned(),
                    children: vec![vec![
                        NodeType::U32(U32Node {
                            value: hdata.read_i32()? as u32,
                            optimized: false,
                        }),
                        NodeType::U8Array(hdata.read_slice(5, false)?),
                    ]],
                };

                let cnode = RecordNode {
                    record_flags: RecordNodeFlags::IS_RECORD_NODE,
                    version: 0,
                    name: COMPRESSED_DATA_TAG.to_owned(),
                    children: vec![vec![
                        NodeType::U8Array(cdata),
                        NodeType::Record(Box::new(hnode)),
                    ]],
                };

                // Replace the full node with the compressed one.
                if let NodeType::Record(ref mut root_node) = self.root_node {
                    if let Some(parent) = root_node.children_mut().get_mut(i1) {
                        if let Some(child) = parent.get_mut(i2) {
                            *child = NodeType::Record(Box::new(cnode));
                        }
                    }
                }
            }
        }

        // Encode the header info, except the offsets, because those are calculated later.
        buffer.write_u32(self.unknown_1)?;
        buffer.write_u32(self.creation_date)?;

        // First, get the strings encoded, as we need to have them in order before encoding the nodes.
        let mut record_names = vec![];
        let mut strings_utf8 = vec![];
        let mut strings_utf16 = vec![];
        Self::read_string_from_node(&self.root_node, &mut record_names, &mut strings_utf8, &mut strings_utf16);

        // Next, encode the nodes. We need them (and the strings) encoded in order to know their offsets.
        let mut nodes_data = vec![];
        save_node(&mut nodes_data, &self.root_node, true, &record_names, &strings_utf8, &strings_utf16)?;

        // Then, encode the strings.
        let mut strings_data: Vec<u8> = vec![];
        strings_data.write_u16(record_names.len() as u16)?;

        // First record names.
        for name in record_names {
            strings_data.write_sized_string_u8(&name)?;
        }

        // Then UTF-16 Strings.
        strings_data.write_u32(strings_utf16.len() as u32)?;
        for (index, string) in strings_utf16.iter().enumerate() {
            strings_data.write_sized_string_u16_u32(string)?;
            strings_data.write_u32(index as u32)?;
        }

        // Then UTF-8 Strings.
        strings_data.write_u32(strings_utf8.len() as u32)?;
        for (index, string) in strings_utf8.iter().enumerate() {
            strings_data.write_sized_string_u8_u32(string)?;
            strings_data.write_u32(index as u32)?;
        }

        // And finally, merge everything.
        buffer.write_u32((12 + nodes_data.len() + 4) as u32)?;
        buffer.write_all(&nodes_data)?;
        buffer.write_all(&strings_data)?;

        if revert_compression {
            *self = backup;
        }

        Ok(())
    }
}

//---------------------------------------------------------------------------//
//                          CBAB node decoding/encoding
//---------------------------------------------------------------------------//
//
// CBAB shares its node encoding with CAAB: the same 7-bit sizes and counts, the same
// optionally-packed record info, and the same string-table references. Only the header and
// the string tables (read by `read_cbab`) differ, which is why the two walkers are separate
// copies of the same logic rather than one function switching on the format.

/// Reads a CBAB node, and everything below it.
pub(crate) fn read_node<R: ReadBytes>(
    data: &mut R,
    is_root_node: bool,
    record_names: &[String],
    strings_utf8: &BTreeMap<u32, String>,
    strings_utf16: &BTreeMap<u32, String>
) -> Result<NodeType> {
    let next_byte = data.read_u8()?;

    if next_byte & RecordNodeFlags::IS_RECORD_NODE.bits() != RecordNodeFlags::IS_RECORD_NODE.bits() {
        return read_leaf(data, next_byte, strings_utf8, strings_utf16);
    }

    let record_flags = RecordNodeFlags::from_bits_truncate(next_byte);
    let has_non_optimized_info =
        record_flags.contains(RecordNodeFlags::HAS_NON_OPTIMIZED_INFO) || is_root_node;

    // Only the root, and records that ask for it, spell out the name index and version.
    let (name_index, version) = if has_non_optimized_info {
        (data.read_u16()?, data.read_u8()?)
    } else {
        let version = (next_byte & 0x1E) >> 1;
        let name_index = (((next_byte & 1) as u16) << 8) + data.read_u8()? as u16;
        (name_index, version)
    };

    let name = record_names.get(name_index as usize)
        .ok_or(RLibError::DecodingESFRecordNameNotFound(name_index))?
        .to_owned();

    let block_size = data.read_cauleb128()?;
    let has_nested_blocks = record_flags.contains(RecordNodeFlags::HAS_NESTED_BLOCKS);
    let group_count = if has_nested_blocks { data.read_cauleb128()? } else { 1 };
    let final_block_offset = data.stream_position()? + block_size as u64;

    let mut children = Vec::with_capacity(group_count as usize);
    for _ in 0..group_count {
        let group_end = if has_nested_blocks {
            let entry_size = data.read_cauleb128()?;
            data.stream_position()? + entry_size as u64
        } else {
            final_block_offset
        };

        let mut nodes = vec![];
        while data.stream_position()? < group_end {
            nodes.push(read_node(data, false, record_names, strings_utf8, strings_utf16)?);
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

    Ok(NodeType::Record(Box::new(RecordNode {
        record_flags,
        version,
        name,
        children,
    })))
}

/// Reads a CBAB leaf node. Strings are looked up in the file's string tables.
fn read_leaf<R: ReadBytes>(
    data: &mut R,
    next_byte: u8,
    strings_utf8: &BTreeMap<u32, String>,
    strings_utf16: &BTreeMap<u32, String>
) -> Result<NodeType> {
    match next_byte {
        INVALID => Err(RLibError::DecodingESFUnsupportedDataType(next_byte)),

        UTF16 => {
            let index = data.read_u32()?;
            match strings_utf16.get(&index) {
                Some(value) => Ok(NodeType::Utf16(value.to_owned())),
                None => Err(RLibError::DecodingESFStringNotFound(index)),
            }
        }
        ASCII => {
            let index = data.read_u32()?;
            match strings_utf8.get(&index) {
                Some(value) => Ok(NodeType::Ascii(value.to_owned())),
                None => Err(RLibError::DecodingESFStringNotFound(index)),
            }
        }

        UTF16_ARRAY => {
            let size = data.read_cauleb128()?;
            let end_offset = data.stream_position()? + size as u64;
            let mut values = vec![];
            while data.stream_position()? < end_offset {
                let index = data.read_u32()?;
                match strings_utf16.get(&index) {
                    Some(value) => values.push(value.to_owned()),
                    None => return Err(RLibError::DecodingESFStringNotFound(index)),
                }
            }
            Ok(NodeType::Utf16Array(values))
        }
        ASCII_ARRAY => {
            let size = data.read_cauleb128()?;
            let end_offset = data.stream_position()? + size as u64;
            let mut values = vec![];
            while data.stream_position()? < end_offset {
                let index = data.read_u32()?;
                match strings_utf8.get(&index) {
                    Some(value) => values.push(value.to_owned()),
                    None => return Err(RLibError::DecodingESFStringNotFound(index)),
                }
            }
            Ok(NodeType::AsciiArray(values))
        }

        UNKNOWN_21 => Ok(NodeType::Unknown21(data.read_u32()?)),
        UNKNOWN_25 => Ok(NodeType::Unknown25(data.read_u32()?)),

        _ => {
            if ESF::is_primitive_array(next_byte) {
                let size = data.read_cauleb128()?;
                let end_offset = data.stream_position()? + size as u64;
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

/// Writes a CBAB node, and everything below it.
pub(crate) fn save_node<W: WriteBytes>(
    buffer: &mut W,
    node: &NodeType,
    is_root_node: bool,
    record_names: &[String],
    strings_utf8: &[String],
    strings_utf16: &[String]
) -> Result<()> {
    if let NodeType::Record(record) = node {
        let name_index = name_index(record_names, record.name())?;

        if record.record_flags().contains(RecordNodeFlags::HAS_NON_OPTIMIZED_INFO) || is_root_node {
            buffer.write_u8(record.record_flags().bits())?;
            buffer.write_u16(name_index)?;
            buffer.write_u8(*record.version())?;
        } else {
            let mut info: u16 = (record.record_flags().bits() as u16) << 8;
            info |= (*record.version() as u16) << 9;
            info |= name_index;

            buffer.write_u16(info.swap_bytes())?;
        }

        let mut children_data = vec![];
        if record.record_flags().contains(RecordNodeFlags::HAS_NESTED_BLOCKS) {
            for group in record.children() {
                let mut group_data = vec![];
                for child in group {
                    save_node(&mut group_data, child, false, record_names, strings_utf8, strings_utf16)?;
                }

                children_data.write_cauleb128(group_data.len() as u32, 0)?;
                children_data.extend_from_slice(&group_data);
            }

            buffer.write_cauleb128(children_data.len() as u32, 0)?;
            buffer.write_cauleb128(record.children().len() as u32, 0)?;
        } else {
            if let Some(group) = record.children().first() {
                for child in group {
                    save_node(&mut children_data, child, false, record_names, strings_utf8, strings_utf16)?;
                }
            }

            buffer.write_cauleb128(children_data.len() as u32, 0)?;
        }

        buffer.write_all(&children_data)?;
        return Ok(());
    }

    if let Some((marker, items)) = ESF::encode_array_items(node)? {
        buffer.write_u8(marker)?;
        buffer.write_cauleb128(items.len() as u32, 0)?;
        buffer.write_all(&items)?;
        return Ok(());
    }

    match node {
        NodeType::Utf16(value) => {
            buffer.write_u8(UTF16)?;
            buffer.write_u32(string_index(strings_utf16, value)?)?;
        }
        NodeType::Ascii(value) => {
            buffer.write_u8(ASCII)?;
            buffer.write_u32(string_index(strings_utf8, value)?)?;
        }
        NodeType::Utf16Array(values) => {
            buffer.write_u8(UTF16_ARRAY)?;
            let mut items = vec![];
            for value in values {
                items.write_u32(string_index(strings_utf16, value)?)?;
            }
            buffer.write_cauleb128(items.len() as u32, 0)?;
            buffer.write_all(&items)?;
        }
        NodeType::AsciiArray(values) => {
            buffer.write_u8(ASCII_ARRAY)?;
            let mut items = vec![];
            for value in values {
                items.write_u32(string_index(strings_utf8, value)?)?;
            }
            buffer.write_cauleb128(items.len() as u32, 0)?;
            buffer.write_all(&items)?;
        }
        NodeType::Unknown21(value) => {
            buffer.write_u8(UNKNOWN_21)?;
            buffer.write_u32(*value)?;
        }
        NodeType::Unknown25(value) => {
            buffer.write_u8(UNKNOWN_25)?;
            buffer.write_u32(*value)?;
        }

        other => {
            if !ESF::save_primitive(buffer, other)? {
                return Err(RLibError::EncodingESFUnsupportedNodeType(format!("{other:?}")));
            }
        }
    }

    Ok(())
}

/// Position of a record name in the name table.
fn name_index(record_names: &[String], name: &str) -> Result<u16> {
    record_names.iter().position(|candidate| candidate == name)
        .map(|index| index as u16)
        .ok_or_else(|| RLibError::EncodingESFStringNotFound(name.to_owned()))
}

/// Position of a string in one of the string tables.
fn string_index(strings: &[String], value: &str) -> Result<u32> {
    strings.iter().position(|candidate| candidate == value)
        .map(|index| index as u32)
        .ok_or_else(|| RLibError::EncodingESFStringNotFound(value.to_owned()))
}
