# rpfm_lib_esf_wasm

A simple WASM wrapper of the [rpfm_lib](https://docs.rs/crate/rpfm_lib/latest) ESF module for decoding and encoding `esf` files in JavaScript.

## Install

Install the library using the command below. There is no NPM package, just pull it directly from github.

```
npm install github:IMRZ/rpfm_lib_esf_wasm#main:dist
```

## Usage

Below is a minimal example using [vite](https://vite.dev/) as the bundler.

```javascript
import exampleFileUrl from "./example.twc?url";
import init, { decode, encode } from "rpfm_lib_esf_wasm";

// Required JavaScript glue code, initialize the WASM module.
await init();

// Load the file and convert it into a `Uint8Array`
const response = await fetch(exampleFileUrl);
const buffer = await response.arrayBuffer();
const bytes = new Uint8Array(binary);

console.log(`Serialized size: ${bytes.length} bytes`);

// Decode the file
const deserialized = decode(bytes);
console.log(deserialized);

// Encode the file back into a `Uint8Array`
const serialized = encode(deserialized);
console.log(`Serialized size: ${serialized.length} bytes`);
```

## Build

```bash
cargo install wasm-pack
wasm-pack build rpfm_lib_esf_wasm --target web
```
