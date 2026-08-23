# nightly feature required

```rust
#![feature(impl_trait_in_assoc_type)]
```

this requirement may be removed in the future (hopefully).

# abs_art

ABStraction of Asynchronous RunTime.

This crates abstracts common APIs, like `JoinHandle` from `compio`, `tokio`, and `smol` so 
that users can write codes across these runtimes with little overhead.

# develop

## how to test
Use `just test` to test on all the supported asynchronous runtimes.
This requires `just` installed in the environment.

See [just](https://github.com/casey/just/blob/master/README.md) for more information.
