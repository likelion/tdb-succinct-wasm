# tdb-succinct-wasm — succinct data structures used by terminus-store-wasm

This repository contains all data structures from
[terminus-store-wasm](https://github.com/likelion/terminus-store-wasm),
as well as the logic for loading and storing them.

This is a synchronous fork of
[tdb-succinct](https://github.com/terminusdb-labs/tdb-succinct),
with async/tokio dependencies removed for WebAssembly compatibility.
