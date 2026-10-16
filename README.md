# rpfm_lib_esf_wasm

A WASM wrapper around the [rpfm_lib](https://docs.rs/crate/rpfm_lib/latest) **ESF module**, providing decode/encode support for `esf` files in JavaScript.

This repository exposes a precompiled WASM build generated via `wasm-pack`.

## Install

There is **no npm package**. Install directly from GitHub using the prebuilt branches:

```sh
# Web/Browser
npm install github:IMRZ/rpfm_lib_esf_wasm#web

# Node.js
npm install github:IMRZ/rpfm_lib_esf_wasm#nodejs
```

## Usage (Web/Browser)

The example below uses [vite](https://vite.dev/). Other bundlers may require less or additional configuration to handle .wasm assets.

```javascript
import init, { decode, encode } from "rpfm_lib_esf_wasm";
import wasm_url from "rpfm_lib_esf_wasm/rpfm_lib_esf_wasm_bg.wasm?url";

// example.twc is an ESF-encoded file
import example_file_url from "./example.twc?url";

// Required JavaScript glue code, initialize the WASM module.
await init(wasm_url);

// Load the file and convert it into a <Uint8Array>
const buffer = await fetch(example_file_url).then((r) => r.arrayBuffer());
const bytes = new Uint8Array(buffer);

// length: 894 bytes
console.log(`length: ${bytes.length} bytes`);

// Decode the file
const decoded = decode(bytes);

// Object { signature: "CBAB", unknown_1: 0, creation_date: 1764961050, root_node: {…} }
console.log(decoded);

// Encode the file back into a <Uint8Array>
const encoded = encode(decoded);

// length: 894 bytes
console.log(`length: ${encoded.length} bytes`);
```

## Usage (Node.js)

```javascript
import fs from "node:fs";
import { decode, encode } from "rpfm_lib_esf_wasm";

// Read file into a Buffer, the Node.js Buffer class is a subclass of JavaScript's <Uint8Array>
const bytes = fs.readFileSync("./example.twc");

const decoded = decode(bytes);

// Object { signature: "CBAB", unknown_1: 0, creation_date: 1764961050, root_node: {…} }
console.log(decoded);

const encoded = encode(decoded);
```

## Lazy decoding

`decode()` decodes the whole node tree in one go. For big files (campaign saves, startpos, ...) that
means decoding every node, even the ones you never look at.

`createReader()` instead parses only the file header and the shared string tables up front, and decodes
nodes on demand. Node ids are opaque strings of the form `"<buffer>:<offset>"`.

```javascript
import { createReader } from "rpfm_lib_esf_wasm";

const reader = createReader(bytes);

// Header info. The node tree is not touched.
reader.signature();       // "CBAB"
reader.creation_date();
const root = reader.root_id();

// Structure. Only children headers are read: grandchildren are skipped, never decoded.
// `children` returns the groups a record is stored in, so it's `[[...]]` for most records.
const groups = reader.children(root);
groups[0][5];             // { id, kind: "record", name: "INNER", version, flags, size }

// Values. Only the node you ask for is decoded.
const value = reader.value(groups[0][1].id);        // a single leaf
const subtree = reader.materialize(groups[0][5].id); // one whole subtree
const raw = reader.chunk(groups[0][4].id);           // raw bytes, no decoding at all

// Path sugar, relative to the root. Add `[n]` for repeated names.
reader.lookup("INNER");      // "<buffer>:<offset>"
reader.lookup("ITEM[1]");    // second record named ITEM
reader.lookup("MISSING");    // null

// Nested ESFs (the COMPRESSED_DATA case) are decompressed on demand too.
const innerRoot = reader.decompress(reader.lookup("COMPRESSED_DATA"));
reader.materialize(innerRoot);   // decoded exactly like decode() would
```

`materialize()` returns the same structure `decode()` returns, so its result can be fed straight to
`encode()`.

## Build

```sh
cargo install wasm-pack

wasm-pack build rpfm_lib_esf_wasm --target web

wasm-pack build rpfm_lib_esf_wasm --target nodejs
```

## Credits and License

This project is based on the ESF module from [`rpfm_lib`](https://github.com/Frodo45127/rpfm), originally developed by **Frodo45127** and contributors.

To enable WebAssembly compilation, the original Rust source was modified and components incompatible with `wasm32-unknown-unknown` were removed or adapted. The core ESF parsing and serialization logic originates from the upstream `rpfm_lib` project.

This repository distributes a modified version of that code compiled to WebAssembly, along with JavaScript bindings.

See the `LICENSE` file for the original license terms.
