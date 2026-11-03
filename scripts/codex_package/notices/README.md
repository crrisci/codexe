# Bundled helper notices

These notices accompany the default ripgrep and V8 helpers in a CodeXe package.
`sources.json` records the source URLs associated with the bundled V8 license
texts; ripgrep notices come from the verified 15.2.0 release archive used by
`../rg`. `pins.json` gates packaging on the ripgrep manifest digest and V8 crate
version. The version gate does not verify that the V8 license texts match the
current source revision.

The workspace currently pins `v8` 152.2.0 from `seanh-oai/rusty_v8` revision
`350c5a859a9b3c84ab8701b3ad51e3b7699b30c0`. The bundled `rusty-v8-LICENSE`
was compared byte-for-byte with that checkout. Its V8 submodule is pinned to
`denoland/v8` revision `c4ca1eccb90c5464d826b7713dc27178a53b0cfe`, which is
recorded in `sources.json`. The bundled `v8-LICENSE` was compared byte-for-byte
with that submodule. No license download is needed during installation.

Project, bubblewrap and other repository notices are copied from their existing
source locations by `../notices.py`. Explicit custom helper binaries supplied to
the package builder must carry any additional notices their distributor requires.
