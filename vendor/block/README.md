# block (vendored)

This is `block` 0.1.6 from <http://github.com/SSheldon/rust-block>, MIT
licensed, copied here so serialX can carry one fix the upstream crate has not
released: `_NSConcreteStackBlock` is declared behind a zero-sized struct rather
than an empty enum, which keeps the crate building once rustc's
`uninhabited_static` lint becomes an error.

The crate is not a direct dependency of serialX; it reaches the build through
gpui-kit's macOS stack (cocoa, metal, core-video). `[patch.crates-io]` in the
root `Cargo.toml` swaps this copy in for the registry release. Drop the patch
once upstream, or the crates that depend on it, move on.
