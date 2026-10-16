//! Read-only checks against ESF files.
//!
//! By default this checks the synthetic fixtures under `tests/fixtures/`. Set `RPFM_ESF_FIXTURES`
//! to a `:`-separated list to check a corpus of real files instead. Anything that is missing, or
//! whose signature is not implemented, is skipped.

use std::io::Cursor;
use std::path::PathBuf;

use rpfm_lib::files::esf::{ESFReader, NodeId, NodeType, ESF};
use rpfm_lib::files::Decodeable;

/// Fixtures checked by default. Missing ones are skipped, and `RPFM_ESF_FIXTURES` replaces the
/// list entirely.
///
/// These are synthetic: the sample ESF with 64 zero bytes appended after its string tables, which
/// is the smallest file that reproduces the padded-file rejection.
const DEFAULT_FIXTURES: [&str; 4] = [
    "fixtures/caab_padded.esf",
    "fixtures/cbab_padded.esf",
    "fixtures/ceab_padded.esf",
    "fixtures/cfab_padded.esf",
];

/// Files to check: the defaults, or a `:`-separated list from `RPFM_ESF_FIXTURES`.
///
/// The env var is how real files can be checked without committing them, e.g.
/// `RPFM_ESF_FIXTURES=/path/a.esf:/path/b.esf cargo test -p rpfm_lib --test esf_files`.
fn fixtures() -> Vec<PathBuf> {
    match std::env::var("RPFM_ESF_FIXTURES") {
        Ok(list) if !list.trim().is_empty() => list.split(':').map(PathBuf::from).collect(),
        _ => DEFAULT_FIXTURES
            .iter()
            .map(|name| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join(name))
            .collect(),
    }
}

/// Reads the record-names offset straight from the raw header (bytes 12..16).
fn names_offset(bytes: &[u8]) -> u64 {
    u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as u64
}

/// Id of the nested-ESF node the eager decoder would swap in, if there is one.
fn nested_compressed(reader: &ESFReader) -> Option<NodeId> {
    reader
        .children(reader.root_id())
        .ok()?
        .first()?
        .last()
        .filter(|stub| stub.name.as_deref() == Some("COMPRESSED_DATA"))
        .map(|stub| stub.id)
}

#[test]
fn fixtures_are_readable() {
    let mut checked = 0;

    for path in fixtures() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };

        println!("=== {}: {} bytes ===", path.display(), bytes.len());

        // A corpus can hold formats this library does not implement, so skip what it cannot open.
        let Ok(mut reader) = ESFReader::new(bytes.clone()) else {
            println!("  skipped: signature not supported");
            continue;
        };

        checked += 1;

        // The eager decoder builds the whole tree, so it is skipped on large files to keep the
        // suite fast. Some real files carry trailing padding after the string tables.
        let eager = if bytes.len() <= 8 * 1024 * 1024 {
            let result = ESF::decode(&mut Cursor::new(bytes.clone()), &None);
            match &result {
                Ok(esf) => println!("  eager: ok, signature {:?}", esf.signature()),
                Err(error) => println!("  eager: rejected -> {error}"),
            }

            result.ok()
        } else {
            println!("  eager: skipped (large file)");
            None
        };

        let signature = reader.signature().to_string();
        println!("  lazy: signature {signature}");

        let root = reader.node(reader.root_id()).expect("root stub failed");
        assert_eq!(root.kind, "record");
        println!("  lazy: root {:?}, size {}", root.name, root.size);

        // The root node must span the whole nodes region: from byte 16 to the name tables.
        assert_eq!(
            root.size,
            names_offset(&bytes) - 16,
            "root size does not cover the nodes region"
        );

        let groups = reader.children(reader.root_id()).expect("children failed");
        let children = groups.first().expect("root has no group 0");
        assert!(!children.is_empty(), "root has no children");
        println!("  lazy: {} top-level children", children.len());

        for child in children.iter().take(12) {
            println!(
                "    {} name={:?} version={:?} size={}",
                child.kind, child.name, child.version, child.size
            );
        }

        // Nested ESFs are decompressed on demand. Only the nested root is checked here:
        // materializing the whole inner tree would dominate the suite.
        let nested = nested_compressed(&reader);
        if let Some(id) = nested {
            let inner = reader.decompress(id).expect("decompress failed");
            let inner_root = reader.node(inner).expect("inner root stub failed");
            println!("  lazy: nested ESF root {:?}", inner_root.name);
            assert_eq!(inner_root.kind, "record");
        }

        // Materialize the whole tree only for small files, and when the eager decoder accepted
        // the file, require the two to agree node for node.
        //
        // The eager decoder replaces a whole file with the ESF nested in its `COMPRESSED_DATA`
        // node, so this can only be asked of files without one.
        if nested.is_none() && root.size <= 1024 * 1024 {
            let lazy_root = reader.materialize(reader.root_id()).expect("materialize failed");

            match &eager {
                Some(esf) => assert_eq!(
                    &lazy_root, esf.root_node(),
                    "lazy decode differs from eager decode"
                ),
                None => assert!(matches!(lazy_root, NodeType::Record(_))),
            }
        }
    }

    if checked == 0 {
        eprintln!("skipping: no fixtures found");
    }
}
