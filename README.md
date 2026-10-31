# Media Weaver

Media Weaver synchronizes image and video metadata dates with a `YEAR/MM` folder structure. It preserves the existing day and time while taking the year and month from the containing folders. If a file has no usable date, it uses the default day and time (`02 12:31:56`).

For images, it writes `DateTimeOriginal`, `CreateDate`, and `ModifyDate`. For videos, it writes the configured QuickTime and XMP date tags. Both media types also get a matching filesystem modification date. A file is skipped only when all configured metadata dates and the filesystem date already match.

## Requirements

- ExifTool installed and available on `PATH`
- Rust toolchain and the Linux system libraries required by Tauri/WebKitGTK

## Build the native Linux app (no Node.js required)

```sh
cargo install tauri-cli --version "^2" --locked
cd src-tauri
cargo tauri build
```

`cargo tauri build` compiles the app in release mode and creates Linux packages. The executable and packages are written under `src-tauri/target/release/` (bundles are in `src-tauri/target/release/bundle/`). To compile the release executable without creating packages, use `cargo tauri build --no-bundle`. Cargo automatically recompiles changed files, so no `--rebuild` flag is needed. The frontend is static HTML/CSS/vanilla JavaScript, so it has no Node-based build step or frontend dependencies. `package.json` and `package-lock.json` are only for the optional npm scripts, which invoke Tauri CLI; they are not needed for the Cargo build or to run the packaged app.

## Run in development for testing

From the project root, start the app in development mode with:

```sh
cd src-tauri
cargo tauri dev
```

This rebuilds the app as needed and lets you test frontend changes without creating a release bundle. Install the Tauri CLI first if needed: `cargo install tauri-cli --version "^2" --locked`.

Choose the folder containing the year directories. The folder layout is `<root>/<YEAR>/<MM...>/<file>`; month folders may have a suffix such as `05-Holiday`. Preview mode is enabled by default. Turn it off only when you are ready to write metadata.

The report updates while processing and lists changed (or would-change), skipped, out-of-structure, and errored files. Recognized media files outside a `<YEAR>/<MM...>` folder pair are counted and shown with an explanation instead of being silently ignored. For a specific malformed EXIF structure, the app reports an error rather than removing all EXIF metadata as a fallback.
