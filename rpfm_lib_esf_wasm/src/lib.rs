use rpfm_lib::files::{Decodeable,Encodeable};
use rpfm_lib::files::esf::ESF;
use std::io::Cursor;
use serde::Serialize;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn decode(input: &[u8]) -> Result<JsValue, JsValue> {
    let mut reader = Cursor::new(input);

    let esf = ESF::decode(&mut reader, &None)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let serializer = serde_wasm_bindgen::Serializer::new()
        .serialize_large_number_types_as_bigints(true);

    esf.serialize(&serializer)
        .map_err(|e| JsValue::from_str(&e.to_string()))
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
