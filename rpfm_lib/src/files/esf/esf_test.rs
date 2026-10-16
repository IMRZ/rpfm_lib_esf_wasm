//! Tests for the ESF decoding/encoding, and for the lazy [`ESFReader`].

use std::io::Cursor;
use std::path::Path;
use std::str::FromStr;

use crate::files::{Decodeable, Encodeable};

use super::*;

//---------------------------------------------------------------------------//
//                                  Helpers
//---------------------------------------------------------------------------//

/// Builds a record node out of its parts.
fn record(name: &str, flags: RecordNodeFlags, version: u8, children: Vec<Vec<NodeType>>) -> NodeType {
    let mut node = RecordNode::default();
    node.set_record_flags(flags);
    node.set_version(version);
    node.set_name(name.to_owned());
    node.set_children(children);

    NodeType::Record(Box::new(node))
}

/// Builds one of the two `ITEM` records, to test repeated names.
fn item(label: &str) -> NodeType {
    record("ITEM", RecordNodeFlags::IS_RECORD_NODE, 1, vec![vec![
        NodeType::Ascii(label.to_owned()),
    ]])
}

/// The sample tree used by both the Rust tests and the JS test suite.
fn sample_esf() -> ESF {
    let mut esf = ESF::default();
    esf.set_signature(ESFSignature::CBAB);
    esf.set_unknown_1(0);
    esf.set_creation_date(1_764_961_050);

    let mut root = RecordNode::default();
    root.set_record_flags(RecordNodeFlags::IS_RECORD_NODE);
    root.set_version(1);
    root.set_name("ROOT".to_owned());
    root.set_children(vec![vec![
        NodeType::Ascii("hello".to_owned()),
        NodeType::U32(U32Node { value: 42, optimized: true }),
        NodeType::Bool(BoolNode { value: true, optimized: true }),
        // `optimized: false` is the canonical form: a non-zero F32 has no optimized marker.
        NodeType::F32(F32Node { value: 1.5, optimized: false }),
        NodeType::U8Array(vec![1, 2, 3, 4, 5]),
        record("INNER", RecordNodeFlags::IS_RECORD_NODE, 2, vec![vec![
            NodeType::I32(I32Node { value: -7, optimized: false }),
            NodeType::Ascii("world".to_owned()),
        ]]),
        record("TAGS", RecordNodeFlags::IS_RECORD_NODE | RecordNodeFlags::HAS_NESTED_BLOCKS, 1, vec![
            vec![NodeType::Ascii("a".to_owned()), NodeType::Ascii("b".to_owned())],
            vec![NodeType::Ascii("c".to_owned())],
        ]),
        item("one"),
        item("two"),
    ]]);
    esf.set_root_node(NodeType::Record(Box::new(root)));

    esf
}

/// A sample tree with a `CAMPAIGN_ENV` node, which makes the encoder store it compressed.
fn sample_esf_compressed() -> ESF {
    let mut esf = ESF::default();
    esf.set_signature(ESFSignature::CBAB);
    esf.set_unknown_1(0);
    esf.set_creation_date(1_764_961_050);

    let mut root = RecordNode::default();
    root.set_record_flags(RecordNodeFlags::IS_RECORD_NODE);
    root.set_version(1);
    root.set_name("ROOT".to_owned());
    root.set_children(vec![vec![
        record("CAMPAIGN_ENV", RecordNodeFlags::IS_RECORD_NODE, 1, vec![vec![
            NodeType::Ascii("payload".to_owned()),
        ]]),
    ]]);
    esf.set_root_node(NodeType::Record(Box::new(root)));

    esf
}

fn encode(esf: &ESF) -> Vec<u8> {
    let mut esf = esf.clone();
    let mut buffer = Vec::new();
    esf.encode(&mut buffer, &None).unwrap();
    buffer
}

fn decode(bytes: &[u8]) -> ESF {
    ESF::decode(&mut Cursor::new(bytes.to_vec()), &None).unwrap()
}

/// Id of the `index`-th child of a node.
fn child_id(reader: &ESFReader, parent: NodeId, index: usize) -> NodeId {
    reader.children(parent).unwrap()[0][index].id
}

//---------------------------------------------------------------------------//
//                                   Tests
//---------------------------------------------------------------------------//

#[test]
fn reader_header_matches_full_decode() {
    let bytes = encode(&sample_esf());
    let decoded = decode(&bytes);
    let reader = ESFReader::new(bytes).unwrap();

    assert_eq!(&reader.signature(), decoded.signature());
    assert_eq!(reader.unknown_1(), *decoded.unknown_1());
    assert_eq!(reader.creation_date(), *decoded.creation_date());
}

#[test]
fn reader_materialize_matches_full_decode() {
    let bytes = encode(&sample_esf());
    let decoded = decode(&bytes);
    let reader = ESFReader::new(bytes).unwrap();

    let root = reader.materialize(reader.root_id()).unwrap();
    assert_eq!(&root, decoded.root_node());
}

#[test]
fn reader_children_are_header_only() {
    let bytes = encode(&sample_esf());
    let reader = ESFReader::new(bytes).unwrap();

    let root = reader.root_id();
    let groups = reader.children(root).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].len(), 9);

    // Leaves have no name, records do.
    assert_eq!(groups[0][0].kind, "ascii");
    assert_eq!(groups[0][0].name, None);
    assert_eq!(groups[0][1].kind, "u32");
    assert_eq!(groups[0][4].kind, "u8_array");

    let inner = &groups[0][5];
    assert_eq!(inner.kind, "record");
    assert_eq!(inner.name.as_deref(), Some("INNER"));
    assert_eq!(inner.version, Some(2));

    // Nested-block records keep their groups.
    let tags = &groups[0][6];
    assert_eq!(tags.name.as_deref(), Some("TAGS"));
    let tag_groups = reader.children(tags.id).unwrap();
    assert_eq!(tag_groups.len(), 2);
    assert_eq!(tag_groups[0].len(), 2);
    assert_eq!(tag_groups[1].len(), 1);
}

#[test]
fn reader_value_decodes_a_single_leaf() {
    let bytes = encode(&sample_esf());
    let decoded = decode(&bytes);
    let reader = ESFReader::new(bytes).unwrap();

    let root = reader.root_id();
    let NodeType::Record(decoded_root) = decoded.root_node() else {
        panic!("the root node should be a record");
    };
    let expected = &decoded_root.children()[0][1];
    let value = reader.value(child_id(&reader, root, 1)).unwrap();

    assert_eq!(&value, expected);
}

#[test]
fn reader_value_rejects_records() {
    let bytes = encode(&sample_esf());
    let reader = ESFReader::new(bytes).unwrap();

    let inner = reader.lookup("INNER").unwrap().unwrap();
    assert!(reader.value(inner).is_err());
}

#[test]
fn reader_lookup_resolves_paths_and_indices() {
    let bytes = encode(&sample_esf());
    let reader = ESFReader::new(bytes).unwrap();

    assert!(reader.lookup("INNER").unwrap().is_some());
    assert!(reader.lookup("TAGS").unwrap().is_some());
    assert!(reader.lookup("MISSING").unwrap().is_none());
    assert!(reader.lookup("INNER/MISSING").unwrap().is_none());

    // The empty path resolves to the root itself.
    assert_eq!(reader.lookup("").unwrap(), Some(reader.root_id()));

    // Repeated names are picked by index, and they are actually different nodes.
    let first = reader.lookup("ITEM").unwrap().unwrap();
    let second = reader.lookup("ITEM[1]").unwrap().unwrap();
    assert_ne!(first, second);

    let first_label = reader.value(child_id(&reader, first, 0)).unwrap();
    let second_label = reader.value(child_id(&reader, second, 0)).unwrap();
    assert_eq!(first_label, NodeType::Ascii("one".to_owned()));
    assert_eq!(second_label, NodeType::Ascii("two".to_owned()));
}

#[test]
fn reader_chunk_returns_raw_bytes() {
    let bytes = encode(&sample_esf());
    let reader = ESFReader::new(bytes).unwrap();

    let blob = child_id(&reader, reader.root_id(), 4);
    let stub = reader.node(blob).unwrap();
    let chunk = reader.chunk(blob).unwrap();

    assert_eq!(stub.kind, "u8_array");
    assert_eq!(chunk.len() as u64, stub.size);
    assert_eq!(chunk[0], U8_ARRAY);
    assert_eq!(&chunk[chunk.len() - 5..], &[1, 2, 3, 4, 5]);
}

#[test]
fn reader_rejects_invalid_node_ids() {
    let bytes = encode(&sample_esf());
    let reader = ESFReader::new(bytes).unwrap();

    assert!("not-an-id".parse::<NodeId>().is_err());
    assert!("0:not-a-number".parse::<NodeId>().is_err());
    assert!(reader.node("9:0".parse().unwrap()).is_err());
}

#[test]
fn reader_decompresses_nested_esf() {
    let bytes = encode(&sample_esf_compressed());
    let decoded = decode(&bytes);
    let mut reader = ESFReader::new(bytes).unwrap();

    // The encoder replaced the `CAMPAIGN_ENV` node with a compressed one.
    let compressed = reader.lookup("COMPRESSED_DATA").unwrap().unwrap();
    assert!(reader.decompress(reader.root_id()).is_err());

    let inner_root = reader.decompress(compressed).unwrap();
    let inner = reader.node(inner_root).unwrap();
    assert_eq!(inner.kind, "record");
    assert_eq!(inner.name.as_deref(), Some("ROOT"));

    // The nested ESF is the same thing a full decode ends up with.
    assert_eq!(&reader.materialize(inner_root).unwrap(), decoded.root_node());
}

#[test]
fn node_id_round_trips_through_strings() {
    let id = NodeId::from_str("3:1234").unwrap();
    assert_eq!(id.to_string(), "3:1234");
    assert_eq!(id, "3:1234".parse::<NodeId>().unwrap());
}

#[test]
fn ceab_round_trips_without_blocks() {
    let mut esf = ESF::default();
    esf.set_signature(ESFSignature::CEAB);
    esf.set_root_node(record("ROOT", RecordNodeFlags::IS_RECORD_NODE, 1, vec![vec![
        NodeType::Ascii("hello".to_owned()),
        NodeType::U32(U32Node { value: 42, optimized: true }),
        NodeType::U8Array(vec![1, 2, 3]),
        record("INNER", RecordNodeFlags::IS_RECORD_NODE, 2, vec![vec![
            NodeType::I32(I32Node { value: -7, optimized: false }),
            NodeType::Ascii("world".to_owned()),
        ]]),
    ]]));

    let bytes = encode(&esf);
    assert_eq!(decode(&bytes).root_node(), esf.root_node());
}

#[test]
fn ceab_round_trips_with_blocks() {
    let mut esf = ESF::default();
    esf.set_signature(ESFSignature::CEAB);
    esf.set_root_node(record("ROOT", RecordNodeFlags::IS_RECORD_NODE, 1, vec![vec![
        record("TAGS", RecordNodeFlags::IS_RECORD_NODE | RecordNodeFlags::HAS_NESTED_BLOCKS, 1, vec![
            vec![NodeType::Ascii("a".to_owned()), NodeType::Ascii("b".to_owned())],
            vec![NodeType::Ascii("c".to_owned())],
        ]),
    ]]));

    let bytes = encode(&esf);
    assert_eq!(decode(&bytes).root_node(), esf.root_node());
}

/// Writes the small padded files under `tests/fixtures/`, which is how they were made.
///
/// Ignored by default because it writes outside the crate, so run it on purpose:
/// `cargo test -p rpfm_lib --lib -- --ignored regenerates_the_padded_fixtures`.
#[test]
#[ignore = "writes files into tests/fixtures/"]
fn regenerates_the_padded_fixtures() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures");

    for signature in [
        ESFSignature::CAAB,
        ESFSignature::CBAB,
        ESFSignature::CEAB,
        ESFSignature::CFAB,
    ] {
        let mut esf = sample_esf();
        esf.set_signature(signature);

        // The padding is the whole point of these files, so it is part of what gets written.
        let mut bytes = encode(&esf);
        bytes.extend_from_slice(&[0u8; 64]);

        let name = format!("{}_padded.esf", signature.to_string().to_lowercase());
        let path = fixtures.join(name);
        std::fs::write(&path, &bytes).unwrap();
        println!("{}: {} bytes", path.display(), bytes.len());
    }
}

#[test]
fn trailing_padding_after_the_tables_is_accepted() {
    for signature in [
        ESFSignature::CAAB,
        ESFSignature::CBAB,
        ESFSignature::CEAB,
        ESFSignature::CFAB,
    ] {
        let mut esf = sample_esf();
        esf.set_signature(signature);
        let bytes = encode(&esf);

        // Real files can carry zero padding after the last section.
        let mut padded = bytes.clone();
        padded.extend_from_slice(&[0u8; 64]);
        ESF::decode(&mut Cursor::new(padded), &None)
            .unwrap_or_else(|error| panic!("{signature} with padding: {error}"));

        // Anything else still means the last section parsed to the wrong place.
        let mut junk = bytes.clone();
        junk.extend_from_slice(&[0xAB; 64]);
        assert!(
            ESF::decode(&mut Cursor::new(junk), &None).is_err(),
            "{signature} with junk after the tables should fail"
        );
    }
}

#[test]
fn every_signature_round_trips() {
    for signature in [
        ESFSignature::CAAB,
        ESFSignature::CBAB,
        ESFSignature::CEAB,
        ESFSignature::CFAB,
    ] {
        let mut esf = sample_esf();
        esf.set_signature(signature);

        let bytes = encode(&esf);
        let decoded = decode(&bytes);

        assert_eq!(
            decoded.root_node(),
            esf.root_node(),
            "eager round-trip differs for {signature}"
        );
        assert_eq!(
            decoded.creation_date(),
            esf.creation_date(),
            "header round-trip differs for {signature}"
        );

        let reader = ESFReader::new(bytes).unwrap();
        assert_eq!(reader.signature(), signature);
        assert_eq!(
            &reader.materialize(reader.root_id()).unwrap(),
            decoded.root_node(),
            "lazy decode differs from eager decode for {signature}"
        );

        // Nav, header-only reads and path lookups work the same in every format.
        assert_eq!(reader.children(reader.root_id()).unwrap()[0].len(), 9);
        assert!(reader.lookup("INNER").unwrap().is_some());
        assert!(reader.lookup("ITEM[1]").unwrap().is_some());
    }
}
