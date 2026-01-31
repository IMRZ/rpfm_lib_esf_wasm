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
