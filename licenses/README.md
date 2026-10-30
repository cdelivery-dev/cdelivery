# Licenses and notices

First-party source code, documentation, scripts, and tests are licensed under [Apache License 2.0](../LICENSE); see [NOTICE](../NOTICE). Third-party components retain their own copyrights and licenses. Rust packages keep `publish = false` and npm packages keep `private: true`; these settings prevent registry publication and do not restrict the open-source license.

## Distribution

The five official images (`cdelivery`, `cdelivery-agent`, `pingress`, `insight`, and `insight-agent`) include `LICENSE`, `NOTICE`, `THIRD_PARTY_LICENSES.txt`, and the toolchain's `RUST_STDLIB_LICENSES.html` under `/usr/share/licenses/cdelivery/`. Base-image notices remain in their original locations, including `/usr/share/doc/`. Preserve the license files alongside binaries when distributing them outside an image.

The CDelivery service serves `/assets/LICENSE.txt` and `/assets/THIRD_PARTY_LICENSES.txt` without authentication; the latter covers its bundled browser assets. Website builds publish `/LICENSE.txt`, `/NOTICE.txt`, and `/THIRD_PARTY_LICENSES.txt`.

## Generate notices

Image builds generate notices automatically from the locked dependencies for their target architecture. To inspect the same output locally, install Bash and jq, then run these commands from the repository root:

```sh
cargo install --locked --version 0.9.3 --features cli cargo-about
cargo deny --locked check licenses
bash scripts/licenses.sh cdelivery x86_64-unknown-linux-gnu
```

Output is written to `target/licenses/<package>/<target>/`. Use `aarch64-unknown-linux-gnu` for arm64 and repeat for each image being distributed. `npm run build` in `website/` generates the website notices from Vite's bundle report and the rendered assets.

## Supplements

`supplements.json` maps exact crate versions to license texts omitted from their published archives. Source links identify the upstream release commit; original notices remain intact. Identical texts share a file. Stalwart copyright declarations come from the same commit's `src/lib.rs`.

`qrcodegen-image` 1.5.2 and `stop-token` 0.7.0 have no separate license file in either the published archive or the release repository. Their entries preserve the license declaration and author metadata from `Cargo.toml`, accompanied by the declared license terms. These entries explicitly distinguish reproduced terms from an upstream license file; no copyright owner or year has been invented.

The Lotus supplement preserves the BSD-3-Clause license missing from its npm package. Website icon and font notices are maintained in `website/public/THIRD_PARTY_LICENSES.txt`.

Review these entries when updating dependencies. `scripts/licenses.sh` fails when a resolved dependency has neither an original license file nor a versioned supplement. Generated reports stay in `target/licenses/`; image builds copy only the distribution files.
