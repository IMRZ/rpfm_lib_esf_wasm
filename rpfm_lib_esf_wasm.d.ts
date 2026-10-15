/* tslint:disable */
/* eslint-disable */

/**
 * A lazily-decoded ESF file.
 *
 * Node ids are opaque strings (of the form `"buffer:offset"`) returned by [`ESFReaderHandle::root_id`],
 * [`ESFReaderHandle::children`] and [`ESFReaderHandle::lookup`]. Pass them back to the other methods.
 */
export class ESFReaderHandle {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Returns the children of a record node, grouped like they are stored in the file.
     *
     * Only the children headers are read: grandchildren are skipped, never decoded.
     */
    children(node_id: string): any;
    /**
     * Returns the raw bytes of a node, without decoding them.
     */
    chunk(node_id: string): Uint8Array;
    /**
     * Returns the creation date of the file.
     */
    creation_date(): number;
    /**
     * Decompresses the nested ESF inside a `COMPRESSED_DATA` record, and returns the id of its
     * root node. All the other reader methods then work on it as usual.
     */
    decompress(node_id: string): string;
    /**
     * Resolves a `/`-separated path of record names into a node id, or `null` if not found.
     *
     * Segments may carry an index for repeated names (`CHARACTER[2]`).
     */
    lookup(path: string): any;
    /**
     * Decodes the whole subtree starting at a node, on demand.
     */
    materialize(node_id: string): any;
    /**
     * Returns the header info of a single node, without decoding its payload.
     */
    node(node_id: string): any;
    /**
     * Returns the id of the root node.
     */
    root_id(): string;
    /**
     * Returns the signature of the file (`"CAAB"`, `"CBAB"`, ...).
     */
    signature(): string;
    /**
     * Returns the `unknown_1` value of the file.
     */
    unknown_1(): number;
    /**
     * Decodes the value of a single node, on demand. Fails if the node is a record.
     */
    value(node_id: string): any;
}

/**
 * Creates a lazy reader over an ESF file.
 *
 * Unlike [`decode`], this does not decode the node tree. Only the file header and the shared
 * string tables are parsed, and the returned reader decodes nodes on demand.
 */
export function create_reader(input: Uint8Array): ESFReaderHandle;

export function decode(input: Uint8Array): any;

export function encode(input: any): Uint8Array;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly decode: (a: number, b: number) => [number, number, number];
    readonly encode: (a: any) => [number, number, number, number];
    readonly create_reader: (a: number, b: number) => [number, number, number];
    readonly __wbg_esfreaderhandle_free: (a: number, b: number) => void;
    readonly esfreaderhandle_signature: (a: number) => [number, number];
    readonly esfreaderhandle_unknown_1: (a: number) => number;
    readonly esfreaderhandle_creation_date: (a: number) => number;
    readonly esfreaderhandle_root_id: (a: number) => [number, number];
    readonly esfreaderhandle_node: (a: number, b: number, c: number) => [number, number, number];
    readonly esfreaderhandle_children: (a: number, b: number, c: number) => [number, number, number];
    readonly esfreaderhandle_value: (a: number, b: number, c: number) => [number, number, number];
    readonly esfreaderhandle_materialize: (a: number, b: number, c: number) => [number, number, number];
    readonly esfreaderhandle_chunk: (a: number, b: number, c: number) => [number, number, number, number];
    readonly esfreaderhandle_lookup: (a: number, b: number, c: number) => [number, number, number];
    readonly esfreaderhandle_decompress: (a: number, b: number, c: number) => [number, number, number, number];
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
