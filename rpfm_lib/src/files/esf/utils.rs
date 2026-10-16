//---------------------------------------------------------------------------//
// Copyright (c) 2017-2024 Ismael Gutiérrez González. All rights reserved.
//
// This file is part of the Rusted PackFile Manager (RPFM) project,
// which can be found here: https://github.com/Frodo45127/rpfm.
//
// This file is licensed under the MIT license, which can be found here:
// https://github.com/Frodo45127/rpfm/blob/master/LICENSE.
//---------------------------------------------------------------------------//

//! Module with generic utils for esf files.

use super::*;

//---------------------------------------------------------------------------//
//                                  Functions
//---------------------------------------------------------------------------//

impl ESF {

    /// This function reads the strings from the provided node and all its children.
    ///
    /// This function is recursive: if you pass it the root node, it'll read all the strings in the ESF file.
    pub(crate) fn read_string_from_node(node_type: &NodeType, record_names: &mut Vec<String>, strings_utf8: &mut Vec<String>, strings_utf16: &mut Vec<String>) {
        match node_type {
            NodeType::Utf16(value) => if !strings_utf16.contains(value) { strings_utf16.push(value.to_owned()) },
            NodeType::Ascii(value) | NodeType::AsciiW21(value) | NodeType::AsciiW25(value) => if !strings_utf8.contains(value) { strings_utf8.push(value.to_owned()) },
            NodeType::Utf16Array(value) => value.iter().for_each(|value| if !strings_utf16.contains(value) { strings_utf16.push(value.to_owned()) }),
            NodeType::AsciiArray(value) => value.iter().for_each(|value| if !strings_utf8.contains(value) { strings_utf8.push(value.to_owned()) }),
            NodeType::Record(value) => {
                if !record_names.contains(&value.name) {
                    record_names.push(value.name.to_owned());
                }
                for node_group in &value.children {
                    for node in node_group {
                        Self::read_string_from_node(node, record_names, strings_utf8, strings_utf16);
                    }
                }
            },

            // Skip any other node.
            _ => {}
        }
    }
}
