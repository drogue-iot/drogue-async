# An embedded-friendly async Executor.

```rust
init_executor!(memory: 1024);

let join_handler = spawn( async move {
  ...
  spawn( async move {
    // nested
  } ).await;
} );
```

## Features

By default it builds for `std` environments, kinda, so that tests can run.
The `std` environment does not actually require `std` but only supports
a never-free'd blob of memory.

Disabling the default features and enabling `cortex-m` will enable a freeable
blob of memory, but currently tasks do not get dropped or freed. It's a TODO.
