<h1 align="center">async-h1b</h1>
<div align="center">
  <strong>
    hitdns fork of Asynchronous HTTP/1.1 parser.
  </strong>
</div>

<br />

<div align="center">
  <!-- Crates version -->
  <a href="https://crates.io/crates/async-h1b">
    <img src="https://img.shields.io/crates/v/async-h1b.svg?style=flat-square"
    alt="Crates.io version" />
  </a>
  <!-- Downloads -->
  <a href="https://crates.io/crates/async-h1b">
    <img src="https://img.shields.io/crates/d/async-h1b.svg?style=flat-square"
      alt="Download" />
  </a>
  <!-- docs.rs docs -->
  <a href="https://docs.rs/async-h1b">
    <img src="https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square"
      alt="docs.rs docs" />
  </a>
</div>

<div align="center">
  <h3>
    <a href="https://docs.rs/async-h1b">
      API Docs
    </a>
    <span> | </span>
    <a href="https://github.com/delta4chat/async-h1/releases">
      Releases
    </a>
    <span> | </span>
    <a href="https://github.com/delta4chat/async-h1/blob/main/.github/CONTRIBUTING.md">
      Contributing
    </a>
  </h3>
</div>

## Installation
```sh
$ cargo add async-h1b
```

## Safety
This crate uses ``#![forbid(unsafe_code)]`` to ensure everything is implemented in
100% Safe Rust.

## Minimum Supported Rust Version

Given the rapidly-improving nature of async Rust, `async-h1b` only
guarantees it will work on the latest stable Rust compiler. Currently
`async-h1b` compiles on `rustc 1.40.0` and above, but we reserve the
right to upgrade the minimum Rust version outside of major
releases. If upgrading stable compiler versions is an issue we
recommend pinning the version of `async-h1b`.

## Contributing
Want to join us? Check out our ["Contributing" guide][contributing] and take a
look at some of these issues:

- [Issues labeled "good first issue"][good-first-issue]
- [Issues labeled "help wanted"][help-wanted]

[contributing]: https://github.com/delta4chat/async-h1/blob/main/.github/CONTRIBUTING.md
[good-first-issue]: https://github.com/delta4chat/async-h1/labels/good%20first%20issue
[help-wanted]: https://github.com/delta4chat/async-h1/labels/help%20wanted

## License

<sup>
Licensed under either of <a href="LICENSE-APACHE">Apache License, Version
2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.
</sup>

<br/>

<sub>
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
</sub>
