# robocad

A simple, fast and lightweight construction tool for creating 3D-printable designs.
Combine primitives and imported STL files, cut holes, group shapes, and export watertight STL files for your slicer.

## Status
Under development. See [PLAN.md](PLAN.md) for the roadmap and [CHANGELOG.md](CHANGELOG.md) for progress.

## Building
Requirements: a stable Rust toolchain (1.90+), CMake and a C++ compiler (for the Manifold geometry kernel).
On Linux you also need the usual windowing development packages (see `.github/workflows/ci.yml`).

```sh
cargo run --release -p rc-app      # desktop app
cargo run --release -p rc-cli -- --help   # command-line tool
```

## Troubleshooting

**`fatal error: 'cstdint' file not found` on macOS** — the Command Line Tools install contains a
stale C++ header directory. Either reinstall them (`sudo rm -rf /Library/Developer/CommandLineTools && xcode-select --install`)
or point the compiler at the SDK headers for the build:

```sh
export CXXFLAGS="-isystem $(xcrun --show-sdk-path)/usr/include/c++/v1"
cargo clean -p manifold-csg-sys   # only if a failed build was cached
cargo build --release
```

The first build clones the Manifold geometry library (pinned commit) and needs network access.

## License
Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
