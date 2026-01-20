# reqwest

[![crates.io](https://img.shields.io/crates/v/reqwest.svg)](https://crates.io/crates/reqwest)
[![Documentation](https://docs.rs/reqwest/badge.svg)](https://docs.rs/reqwest)
[![MIT/Apache-2 licensed](https://img.shields.io/crates/l/reqwest.svg)](./LICENSE-APACHE)
[![CI](https://github.com/seanmonstar/reqwest/actions/workflows/ci.yml/badge.svg)](https://github.com/seanmonstar/reqwest/actions/workflows/ci.yml)

An ergonomic, batteries-included HTTP Client for Rust.

- Async and blocking `Client`s
- Plain bodies, JSON, urlencoded, multipart
- Customizable redirect policy
- HTTP Proxies
- HTTPS via rustls (or optionally, system-native TLS)
- Cookie Store
- WASM


## Example

This asynchronous example uses [Tokio](https://tokio.rs) and enables some
optional features, so your `Cargo.toml` could look like this:

```toml
[dependencies]
reqwest = { version = "0.13", features = ["json"] }
tokio = { version = "1", features = ["full"] }
```

And then the code:

```rust,no_run
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let resp = reqwest::get("https://httpbin.org/ip")
        .await?
        .json::<HashMap<String, String>>()
        .await?;
    println!("{resp:#?}");
    Ok(())
}
```

## Commercial Support

For private advice, support, reviews, access to the maintainer, and the like, reach out for [commercial support][sponsor].

## Requirements

By default, Reqwest uses [rustls](https://github.com/rustls/rustls), but when the `native-tls` feature is enabled
it will use the operating system TLS framework if available, meaning Windows and macOS.
On Linux, it will use the available OpenSSL (see https://docs.rs/openssl for supported versions and more details)
or fail to build if not found. Alternatively you can enable the `native-tls-vendored` feature to compile a copy of OpenSSL.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.

## Sponsors

Support this project by becoming a [sponsor][].

[sponsor]: https://seanmonstar.com/sponsor

## Fork Modification

This fork gives you an option to serialize responses and catch and store requests and responses in sqlite database to use them later.

### Development History

I used a custom library on top of httpx called httpc that has some helpful utilities and an `httpc.catcher` extension.
The extension sits in front of the request handler so that it can catch a request and provide a response by using the cached one instead of actually sending network requests, or store responses for later use.
I wanted to port it to reqwest as well.

[reqwest_mock](https://github.com/leoschwarz/reqwest_mock) provides wrapper trait aroud `reqwest::Client`, so others can implement mocking client for reqwest. But it is discountinued because the library author thought there are much better solutions out there. But it provides useful links of mocking libraries of reqwest.

I tried to implement a mock client by implementing a custom middleware for Tower ([guide](https://github.com/tower-rs/tower/blob/master/guides/building-a-middleware-from-scratch.md)), but I couldn't manipulate a request and a response with it.

So I tried using [reqwest-middleware](https://github.com/TrueLayer/reqwest-middleware) to implement the mocker, but it was not enuogh to implement what I want, although it can be a middleware between a request and a response, I cannot find a way to crate a custom response or storing a body of the response after being initialized.

Finally, I decided to fork reqwest itself and make own version of reqwest that supports what I want.
