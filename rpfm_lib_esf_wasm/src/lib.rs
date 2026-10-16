use rpfm_lib::files::{Decodeable,Encodeable};
use rpfm_lib::files::esf::{ESF, ESFReader, NodeId};
use std::io::Cursor;
use std::str::FromStr;
use serde::Serialize;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn decode(input: &[u8]) -> Result<JsValue, JsValue> {
    let mut reader = Cursor::new(input);

    let esf = ESF::decode(&mut reader, &None)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    esf.serialize(&value_serializer())
        .map_err(js_error)
}

#[wasm_bindgen]
pub fn encode(input: JsValue) -> Result<Vec<u8>, JsValue> {
    let mut esf: ESF = serde_wasm_bindgen::from_value(input)
        .map_err(|e| JsValue::from_str(&format!("Deserialization failed: {}", e)))?;

    let mut buffer = Vec::new();

    esf.encode(&mut buffer, &None)
        .map_err(|e| JsValue::from_str(&format!("Encoding failed: {}", e)))?;

    Ok(buffer)
}

/// Creates a lazy reader over an ESF file.
///
/// Unlike [`decode`], this does not decode the node tree. Only the file header and the shared
/// string tables are parsed, and the returned reader decodes nodes on demand.
#[wasm_bindgen]
pub fn create_reader(input: &[u8]) -> Result<ESFReaderHandle, JsValue> {
    let reader = ESFReader::new(input.to_vec()).map_err(js_error)?;
    Ok(ESFReaderHandle { reader })
}

/// A lazily-decoded ESF file.
///
/// Node ids are opaque strings (of the form `"buffer:offset"`) returned by [`ESFReaderHandle::root_id`],
/// [`ESFReaderHandle::children`] and [`ESFReaderHandle::lookup`]. Pass them back to the other methods.
#[wasm_bindgen]
pub struct ESFReaderHandle {
    reader: ESFReader,
}

#[wasm_bindgen]
impl ESFReaderHandle {

    /// Returns the signature of the file (`"CAAB"`, `"CBAB"`, ...).
    pub fn signature(&self) -> String {
        self.reader.signature().to_string()
    }

    /// Returns the `unknown_1` value of the file.
    pub fn unknown_1(&self) -> u32 {
        self.reader.unknown_1()
    }

    /// Returns the creation date of the file.
    pub fn creation_date(&self) -> u32 {
        self.reader.creation_date()
    }

    /// Returns the id of the root node.
    pub fn root_id(&self) -> String {
        self.reader.root_id().to_string()
    }

    /// Returns the header info of a single node, without decoding its payload.
    pub fn node(&self, node_id: &str) -> Result<JsValue, JsValue> {
        let stub = self.reader.node(parse_node_id(node_id)?).map_err(js_error)?;
        serde_wasm_bindgen::to_value(&stub).map_err(js_error)
    }

    /// Returns the children of a record node, grouped like they are stored in the file.
    ///
    /// Only the children headers are read: grandchildren are skipped, never decoded.
    pub fn children(&self, node_id: &str) -> Result<JsValue, JsValue> {
        let groups = self.reader.children(parse_node_id(node_id)?).map_err(js_error)?;
        serde_wasm_bindgen::to_value(&groups).map_err(js_error)
    }

    /// Decodes the value of a single node, on demand. Fails if the node is a record.
    pub fn value(&self, node_id: &str) -> Result<JsValue, JsValue> {
        let node = self.reader.value(parse_node_id(node_id)?).map_err(js_error)?;
        node.serialize(&value_serializer()).map_err(js_error)
    }

    /// Decodes the whole subtree starting at a node, on demand.
    pub fn materialize(&self, node_id: &str) -> Result<JsValue, JsValue> {
        let node = self.reader.materialize(parse_node_id(node_id)?).map_err(js_error)?;
        node.serialize(&value_serializer()).map_err(js_error)
    }

    /// Returns the raw bytes of a node, without decoding them.
    pub fn chunk(&self, node_id: &str) -> Result<Vec<u8>, JsValue> {
        self.reader.chunk(parse_node_id(node_id)?).map_err(js_error)
    }

    /// Resolves a `/`-separated path of record names into a node id, or `null` if not found.
    ///
    /// Segments may carry an index for repeated names (`CHARACTER[2]`).
    pub fn lookup(&self, path: &str) -> Result<JsValue, JsValue> {
        let found = self
            .reader
            .lookup(path)
            .map_err(js_error)?
            .map(|id| id.to_string());

        serde_wasm_bindgen::to_value(&found).map_err(js_error)
    }

    /// Decompresses the nested ESF inside a `COMPRESSED_DATA` record, and returns the id of its
    /// root node. All the other reader methods then work on it as usual.
    pub fn decompress(&mut self, node_id: &str) -> Result<String, JsValue> {
        let id = self.reader.decompress(parse_node_id(node_id)?).map_err(js_error)?;
        Ok(id.to_string())
    }
}

fn parse_node_id(node_id: &str) -> Result<NodeId, JsValue> {
    NodeId::from_str(node_id).map_err(js_error)
}

fn value_serializer() -> serde_wasm_bindgen::Serializer {
    serde_wasm_bindgen::Serializer::new()
        .serialize_large_number_types_as_bigints(true)
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
