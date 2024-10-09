# abs_art

ABStraction of Asynchronous RunTime.

This crates abstracts common APIs, like `JoinHandle` from `tokio`, `async-std` and `smol` so that users can write codes across these runtimes with little overhead.