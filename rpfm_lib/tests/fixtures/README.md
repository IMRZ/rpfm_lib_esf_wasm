# Fixtures

Synthetic ESF files used by `rpfm_lib/tests/esf_files.rs`, one per signature and named after it.
Nothing here comes from a game.

| File | Signature | Size |
| --- | --- | --- |
| `caab_padded.esf` | CAAB | 251 B |
| `cbab_padded.esf` | CBAB | 265 B |
| `ceab_padded.esf` | CEAB | 222 B |
| `cfab_padded.esf` | CFAB | 286 B |

Each one is the sample ESF from `esf_test.rs` encoded by that signature's writer, plus 64 zero
bytes appended after the string tables. Remove the last 64 bytes and the file decodes; leave them
in place and the string-table check used to reject the file, because it landed 64 bytes short of
the end of the stream. That is the whole reproduction, and it needs no real file: some shipped
CAAB startpos files carry the same kind of padding, in one case 2,216 bytes of it.

Regenerate them with:

```sh
cargo test -p rpfm_lib --lib -- --ignored regenerates_the_padded_fixtures
```

To check the decoders against real files without committing them, point the test at a corpus:

```sh
RPFM_ESF_FIXTURES=$(find <dir> -iname '*.esf' | paste -sd:) \
  cargo test -p rpfm_lib --test esf_files -- --nocapture
```
