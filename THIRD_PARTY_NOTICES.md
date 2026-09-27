# Third-party notices

Amazing is written in Rust and std-only in its engine cores. Some algorithms are ported
(copied and adapted, not linked) from other open-source projects under permissive licenses,
per the porting policy in `AGENTS.md`. Each upstream is listed here with its license and the
files of ours that contain ported code; those files also carry a header comment naming the
upstream commit.

Only MIT and Apache-2.0 sources may be ported. The full license texts are reproduced below
the table, once per license.

| Upstream | Commit | License | Copyright | Our files |
|----------|--------|---------|-----------|-----------|
| [lucid-softworks/lumen](https://github.com/lucid-softworks/lumen) | `f6c9860d44fa52249cfd647122edea184d32c863` | MIT | Copyright (c) 2026 Lucid Softworks limited | `crates/amazing-js/src/interp.rs`, `crates/amazing-js/src/interp_disposable.rs`, `crates/amazing-js/src/value.rs` |
| [boa-dev/temporal](https://github.com/boa-dev/temporal) (temporal_rs) | `eadf8039d2577e4375493a0eb3f46a66f25616ec` | Apache-2.0 (dual MIT/Apache-2.0 upstream) | Copyright (c) 2024 Boa | `crates/amazing-js/src/builtins_temporal_arithmetic.rs` |
| [lucid-softworks/lumen](https://github.com/lucid-softworks/lumen) | `f6c9860d44fa52249cfd647122edea184d32c863` | MIT | Copyright (c) 2026 Lucid Softworks limited | `crates/amazing-js/src/interp.rs`, `crates/amazing-js/src/interp_stmt.rs`, `crates/amazing-js/src/interp_disposable.rs`, `crates/amazing-js/src/value.rs` |

## MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy of this software
and associated documentation files (the "Software"), to deal in the Software without
restriction, including without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## Apache License 2.0

Ported Apache-2.0 code is used under the Apache License, Version 2.0
(https://www.apache.org/licenses/LICENSE-2.0). Any upstream `NOTICE` file content is
reproduced in the table row for that upstream.
