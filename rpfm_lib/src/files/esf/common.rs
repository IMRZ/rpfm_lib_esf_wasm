//! Format-neutral ESF code.
//!
//! Some parts of an ESF node decode identically in every format: the leaf primitives, and the
//! items of a numeric array. Those live here so the formats do not each keep their own copy.
//!
//! Anything that *does* vary between formats -- sizes and counts, record info, strings -- is
//! deliberately not here. Each format's own module owns that.

use std::io::{SeekFrom, Write};

use super::*;

impl ESF {

    /// Reads a primitive leaf node, if the marker is one.
    ///
    /// Returns `None` when the marker is not a format-neutral primitive, which includes the
    /// markers whose meaning changes between formats (strings, arrays, `0x21`/`0x25`).
    pub(crate) fn read_primitive<R: ReadBytes>(data: &mut R, next_byte: u8) -> Result<Option<NodeType>> {
        let node = match next_byte {
            BOOL => NodeType::Bool(BoolNode { value: data.read_bool()?, optimized: false }),
            I8 => NodeType::I8(data.read_i8()?),
            I16 => NodeType::I16(data.read_i16()?),
            I32 => NodeType::I32(I32Node { value: data.read_i32()?, optimized: false }),
            I64 => NodeType::I64(data.read_i64()?),
            U8 => NodeType::U8(data.read_u8()?),
            U16 => NodeType::U16(data.read_u16()?),
            U32 => NodeType::U32(U32Node { value: data.read_u32()?, optimized: false }),
            U64 => NodeType::U64(data.read_u64()?),
            F32 => NodeType::F32(F32Node { value: data.read_f32()?, optimized: false }),
            F64 => NodeType::F64(data.read_f64()?),
            COORD_2D => NodeType::Coord2d(Coordinates2DNode { x: data.read_f32()?, y: data.read_f32()? }),
            COORD_3D => NodeType::Coord3d(Coordinates3DNode { x: data.read_f32()?, y: data.read_f32()?, z: data.read_f32()? }),
            ANGLE => NodeType::Angle(data.read_i16()?),

            BOOL_TRUE => NodeType::Bool(BoolNode { value: true, optimized: true }),
            BOOL_FALSE => NodeType::Bool(BoolNode { value: false, optimized: true }),
            U32_ZERO => NodeType::U32(U32Node { value: 0, optimized: true }),
            U32_ONE => NodeType::U32(U32Node { value: 1, optimized: true }),
            U32_BYTE => NodeType::U32(U32Node { value: data.read_u8()? as u32, optimized: true }),
            U32_16BIT => NodeType::U32(U32Node { value: data.read_u16()? as u32, optimized: true }),
            U32_24BIT => NodeType::U32(U32Node { value: data.read_u24()?, optimized: true }),
            I32_ZERO => NodeType::I32(I32Node { value: 0, optimized: true }),
            I32_BYTE => NodeType::I32(I32Node { value: data.read_i8()? as i32, optimized: true }),
            I32_16BIT => NodeType::I32(I32Node { value: data.read_i16()? as i32, optimized: true }),
            I32_24BIT => NodeType::I32(I32Node { value: data.read_i24()?, optimized: true }),
            F32_ZERO => NodeType::F32(F32Node { value: 0.0, optimized: true }),

            UNKNOWN_23 => NodeType::Unknown23(data.read_u8()?),
            UNKNOWN_24 => NodeType::Unknown24(data.read_u16()?),
            UNKNOWN_26 => {
                let mut node_data = vec![];
                let first_byte = data.read_u8()?;
                node_data.push(first_byte);

                if first_byte % 8 == 0 && first_byte != 0 {
                    node_data.extend_from_slice(&data.read_slice(first_byte as usize, false)?);
                } else {
                    node_data.extend_from_slice(&data.read_slice(7, false)?);
                }

                let last_byte = data.read_u8();
                if last_byte.is_ok() && last_byte.unwrap() != 0x9C {
                    data.seek(SeekFrom::Current(-1))?;
                }

                NodeType::Unknown26(node_data)
            }

            _ => return Ok(None),
        };

        Ok(Some(node))
    }

    /// Skips a primitive leaf's payload, if the marker is one.
    ///
    /// Returns `false` when the marker is not a format-neutral primitive.
    pub(crate) fn skip_primitive<R: ReadBytes>(data: &mut R, next_byte: u8) -> Result<bool> {
        match next_byte {
            BOOL | I8 | U8 | UNKNOWN_23 => { data.read_u8()?; }
            I16 | U16 | ANGLE | UNKNOWN_24 => { data.read_u16()?; }
            I32 | U32 | F32 => { data.read_u32()?; }
            I64 | U64 | F64 => { data.read_u64()?; }
            COORD_2D => { data.read_slice(8, false)?; }
            COORD_3D => { data.read_slice(12, false)?; }

            BOOL_TRUE | BOOL_FALSE | U32_ZERO | U32_ONE | I32_ZERO | F32_ZERO => {}
            U32_BYTE | I32_BYTE => { data.read_u8()?; }
            U32_16BIT | I32_16BIT => { data.read_u16()?; }
            U32_24BIT | I32_24BIT => { data.read_u24()?; }

            UNKNOWN_26 => {
                let first_byte = data.read_u8()?;
                if first_byte % 8 == 0 && first_byte != 0 {
                    data.read_slice(first_byte as usize, false)?;
                } else {
                    data.read_slice(7, false)?;
                }

                if let Ok(last_byte) = data.read_u8() {
                    if last_byte != 0x9C {
                        data.seek(SeekFrom::Current(-1))?;
                    }
                }
            }

            _ => return Ok(false),
        }

        Ok(true)
    }

    /// Writes a primitive leaf node, if the node is one. Returns `false` otherwise.
    pub(crate) fn save_primitive<W: WriteBytes>(buffer: &mut W, node: &NodeType) -> Result<bool> {
        match node {
            NodeType::Bool(value) => {
                if *value.optimized() {
                    buffer.write_u8(if *value.value() { BOOL_TRUE } else { BOOL_FALSE })?;
                } else {
                    buffer.write_u8(BOOL)?;
                    buffer.write_bool(*value.value())?;
                }
            }
            NodeType::I8(value) => { buffer.write_u8(I8)?; buffer.write_i8(*value)?; }
            NodeType::I16(value) => { buffer.write_u8(I16)?; buffer.write_i16(*value)?; }
            NodeType::I32(value) => {
                if *value.optimized() {
                    let value = *value.value();
                    if value == 0 {
                        buffer.write_u8(I32_ZERO)?;
                    } else if value.is_positive() && value <= i8::MAX as i32 {
                        buffer.write_u8(I32_BYTE)?;
                        buffer.write_i8(value as i8)?;
                    } else if value.is_positive() && value <= i16::MAX as i32 {
                        buffer.write_u8(I32_16BIT)?;
                        buffer.write_i16(value as i16)?;
                    } else if value.is_positive() && value <= 8_388_607 {
                        buffer.write_u8(I32_24BIT)?;
                        buffer.write_i24(value)?;
                    } else if value >= i8::MIN as i32 && value < 0 {
                        buffer.write_u8(I32_BYTE)?;
                        buffer.write_i8(value as i8)?;
                    } else if value >= i16::MIN as i32 && value < 0 {
                        buffer.write_u8(I32_16BIT)?;
                        buffer.write_i16(value as i16)?;
                    } else if (-8_388_608..0).contains(&value) {
                        buffer.write_u8(I32_24BIT)?;
                        buffer.write_i24(value)?;
                    } else {
                        buffer.write_u8(I32)?;
                        buffer.write_i32(value)?;
                    }
                } else {
                    buffer.write_u8(I32)?;
                    buffer.write_i32(*value.value())?;
                }
            }
            NodeType::I64(value) => { buffer.write_u8(I64)?; buffer.write_i64(*value)?; }
            NodeType::U8(value) => { buffer.write_u8(U8)?; buffer.write_u8(*value)?; }
            NodeType::U16(value) => { buffer.write_u8(U16)?; buffer.write_u16(*value)?; }
            NodeType::U32(value) => {
                if *value.optimized() {
                    let value = *value.value();
                    if value == 0 {
                        buffer.write_u8(U32_ZERO)?;
                    } else if value == 1 {
                        buffer.write_u8(U32_ONE)?;
                    } else if value <= 0xFF {
                        buffer.write_u8(U32_BYTE)?;
                        buffer.write_u8(value as u8)?;
                    } else if value <= 0xFFFF {
                        buffer.write_u8(U32_16BIT)?;
                        buffer.write_u16(value as u16)?;
                    } else if value <= 0xFFFFFF {
                        buffer.write_u8(U32_24BIT)?;
                        buffer.write_u24(value)?;
                    } else {
                        buffer.write_u8(U32)?;
                        buffer.write_u32(value)?;
                    }
                } else {
                    buffer.write_u8(U32)?;
                    buffer.write_u32(*value.value())?;
                }
            }
            NodeType::U64(value) => { buffer.write_u8(U64)?; buffer.write_u64(*value)?; }
            NodeType::F32(value) => {
                if *value.optimized() && (*value.value() - 0.0).abs() < f32::EPSILON {
                    buffer.write_u8(F32_ZERO)?;
                } else {
                    buffer.write_u8(F32)?;
                    buffer.write_f32(*value.value())?;
                }
            }
            NodeType::F64(value) => { buffer.write_u8(F64)?; buffer.write_f64(*value)?; }

            NodeType::Coord2d(value) => {
                buffer.write_u8(COORD_2D)?;
                buffer.write_f32(value.x)?;
                buffer.write_f32(value.y)?;
            }
            NodeType::Coord3d(value) => {
                buffer.write_u8(COORD_3D)?;
                buffer.write_f32(value.x)?;
                buffer.write_f32(value.y)?;
                buffer.write_f32(value.z)?;
            }
            NodeType::Angle(value) => { buffer.write_u8(ANGLE)?; buffer.write_i16(*value)?; }

            NodeType::Unknown23(value) => { buffer.write_u8(UNKNOWN_23)?; buffer.write_u8(*value)?; }
            NodeType::Unknown24(value) => { buffer.write_u8(UNKNOWN_24)?; buffer.write_u16(*value)?; }
            NodeType::Unknown26(value) => { buffer.write_u8(UNKNOWN_26)?; buffer.write_all(value)?; }

            _ => return Ok(false),
        }

        Ok(true)
    }

    /// Returns whether the marker is a numeric array, i.e. an array whose items decode the
    /// same in every format. String arrays are excluded, because they do not.
    pub(crate) fn is_primitive_array(next_byte: u8) -> bool {
        matches!(next_byte,
            BOOL_ARRAY | I8_ARRAY | I16_ARRAY | I32_ARRAY | I64_ARRAY | U8_ARRAY | U16_ARRAY
            | U32_ARRAY | U64_ARRAY | F32_ARRAY | F64_ARRAY | COORD_2D_ARRAY | COORD_3D_ARRAY
            | ANGLE_ARRAY | U32_BYTE_ARRAY | U32_16BIT_ARRAY | U32_24BIT_ARRAY
            | I32_BYTE_ARRAY | I32_16BIT_ARRAY | I32_24BIT_ARRAY
        )
    }

    /// Reads the items of a numeric array, once its end offset is known.
    ///
    /// Returns `None` when the marker is not a numeric array.
    pub(crate) fn read_array_items<R: ReadBytes>(
        data: &mut R,
        next_byte: u8,
        end_offset: u64,
    ) -> Result<Option<NodeType>> {
        // Every numeric array is a run of fixed-size values filling the marked span.
        macro_rules! fill {
            ($read:expr, $variant:expr) => {{
                let mut values = vec![];
                while data.stream_position()? < end_offset {
                    values.push($read);
                }
                $variant(values)
            }};
        }

        let node = match next_byte {
            BOOL_ARRAY => fill!(data.read_bool()?, NodeType::BoolArray),
            I16_ARRAY => fill!(data.read_i16()?, NodeType::I16Array),
            I32_ARRAY => fill!(data.read_i32()?, |v| NodeType::I32Array(VecI32Node { value: v, optimized: false })),
            I64_ARRAY => fill!(data.read_i64()?, NodeType::I64Array),
            U16_ARRAY => fill!(data.read_u16()?, NodeType::U16Array),
            U32_ARRAY => fill!(data.read_u32()?, |v| NodeType::U32Array(VecU32Node { value: v, optimized: false })),
            U64_ARRAY => fill!(data.read_u64()?, NodeType::U64Array),
            F32_ARRAY => fill!(data.read_f32()?, NodeType::F32Array),
            F64_ARRAY => fill!(data.read_f64()?, NodeType::F64Array),
            COORD_2D_ARRAY => fill!(Coordinates2DNode { x: data.read_f32()?, y: data.read_f32()? }, NodeType::Coord2dArray),
            COORD_3D_ARRAY => fill!(Coordinates3DNode { x: data.read_f32()?, y: data.read_f32()?, z: data.read_f32()? }, NodeType::Coord3dArray),
            ANGLE_ARRAY => fill!(data.read_i16()?, NodeType::AngleArray),

            U32_BYTE_ARRAY => fill!(data.read_u8()? as u32, |v| NodeType::U32Array(VecU32Node { value: v, optimized: true })),
            U32_16BIT_ARRAY => fill!(data.read_u16()? as u32, |v| NodeType::U32Array(VecU32Node { value: v, optimized: true })),
            U32_24BIT_ARRAY => fill!(data.read_u24()?, |v| NodeType::U32Array(VecU32Node { value: v, optimized: true })),
            I32_BYTE_ARRAY => fill!(data.read_i8()? as i32, |v| NodeType::I32Array(VecI32Node { value: v, optimized: true })),
            I32_16BIT_ARRAY => fill!(data.read_i16()? as i32, |v| NodeType::I32Array(VecI32Node { value: v, optimized: true })),
            I32_24BIT_ARRAY => fill!(data.read_i24()?, |v| NodeType::I32Array(VecI32Node { value: v, optimized: true })),

            // Byte arrays are stored as one raw run rather than item by item.
            I8_ARRAY | U8_ARRAY => {
                let size = (end_offset - data.stream_position()?) as usize;
                let bytes = data.read_slice(size, false)?;
                if next_byte == I8_ARRAY { NodeType::I8Array(bytes) } else { NodeType::U8Array(bytes) }
            }

            _ => return Ok(None),
        };

        Ok(Some(node))
    }

    /// Encodes the marker and items of a numeric array, without its size field.
    ///
    /// Returns `None` when the node is not a numeric array.
    pub(crate) fn encode_array_items(node: &NodeType) -> Result<Option<(u8, Vec<u8>)>> {
        let mut items = Vec::new();

        let marker = match node {
            NodeType::BoolArray(values) => {
                for value in values { items.write_bool(*value)?; }
                BOOL_ARRAY
            }
            NodeType::I8Array(values) => { items.write_all(values)?; I8_ARRAY }
            NodeType::I16Array(values) => { for value in values { items.write_i16(*value)?; } I16_ARRAY }
            NodeType::I32Array(values) => {
                if *values.optimized() {
                    let max = values.value().iter().map(|value| value.abs()).max().unwrap_or(0);
                    if max <= i8::MAX as i32 {
                        for value in values.value() { items.write_i8(*value as i8)?; }
                        I32_BYTE_ARRAY
                    } else if max <= i16::MAX as i32 {
                        for value in values.value() { items.write_i16(*value as i16)?; }
                        I32_16BIT_ARRAY
                    } else if max <= 8_388_607 {
                        for value in values.value() { items.write_i24(*value)?; }
                        I32_24BIT_ARRAY
                    } else {
                        for value in values.value() { items.write_i32(*value)?; }
                        I32_ARRAY
                    }
                } else {
                    for value in values.value() { items.write_i32(*value)?; }
                    I32_ARRAY
                }
            }
            NodeType::I64Array(values) => { for value in values { items.write_i64(*value)?; } I64_ARRAY }
            NodeType::U8Array(values) => { items.write_all(values)?; U8_ARRAY }
            NodeType::U16Array(values) => { for value in values { items.write_u16(*value)?; } U16_ARRAY }
            NodeType::U32Array(values) => {
                if *values.optimized() {
                    let max = values.value().iter().copied().max().unwrap_or(0);
                    if max <= 0xFF {
                        for value in values.value() { items.write_u8(*value as u8)?; }
                        U32_BYTE_ARRAY
                    } else if max <= 0xFFFF {
                        for value in values.value() { items.write_u16(*value as u16)?; }
                        U32_16BIT_ARRAY
                    } else if max <= 0xFFFFFF {
                        for value in values.value() { items.write_u24(*value)?; }
                        U32_24BIT_ARRAY
                    } else {
                        for value in values.value() { items.write_u32(*value)?; }
                        U32_ARRAY
                    }
                } else {
                    for value in values.value() { items.write_u32(*value)?; }
                    U32_ARRAY
                }
            }
            NodeType::U64Array(values) => { for value in values { items.write_u64(*value)?; } U64_ARRAY }
            NodeType::F32Array(values) => { for value in values { items.write_f32(*value)?; } F32_ARRAY }
            NodeType::F64Array(values) => { for value in values { items.write_f64(*value)?; } F64_ARRAY }
            NodeType::Coord2dArray(values) => {
                for value in values { items.write_f32(value.x)?; items.write_f32(value.y)?; }
                COORD_2D_ARRAY
            }
            NodeType::Coord3dArray(values) => {
                for value in values { items.write_f32(value.x)?; items.write_f32(value.y)?; items.write_f32(value.z)?; }
                COORD_3D_ARRAY
            }
            NodeType::AngleArray(values) => { for value in values { items.write_i16(*value)?; } ANGLE_ARRAY }

            _ => return Ok(None),
        };

        Ok(Some((marker, items)))
    }

    /// Returns whether the rest of the stream is only zero padding.
    ///
    /// Real files can carry zero padding after the string tables, and the reference
    /// implementation simply ignores whatever follows them. Treating zeros as padding keeps
    /// those files readable, while still catching a table parse that ends in the middle of
    /// real data.
    pub(crate) fn only_padding_remains<R: ReadBytes>(data: &mut R) -> Result<bool> {
        let remaining = data.len()? - data.stream_position()?;
        if remaining == 0 {
            return Ok(true);
        }

        let bytes = data.read_slice(remaining as usize, false)?;
        Ok(bytes.iter().all(|byte| *byte == 0))
    }
}
