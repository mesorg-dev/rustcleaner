# RustCleaner

RustCleaner is a Rust + Slint prototype for reviewing files before cleanup.

## Current features

- Choose a folder and scan it recursively.
- Detect duplicates by comparing file contents (hashing only files with matching sizes).
- Flag known installer packages of 100 MiB or larger.
- Search by name/path, filter by extension, and sort by extension.
- Select individual files or all scanned files. Deletion requires a second confirmation.
- Scan and deletion run off the UI thread; filesystem errors are shown in the status area.
- Symbolic links are not followed, and deletion only removes regular files.

## Run

Install the Rust toolchain, then run:

```sh
cargo run
```

Run the scanner tests with:

```sh
cargo test
```

## Platform status

### Desktop

The desktop build uses the native folder picker. Deleted files are moved to the system trash.

### Android

The Android build uses the Storage Access Framework (SAF). The user explicitly chooses a folder,
and the app persists access only to that tree. It can recursively list documents, hash same-size
files to identify duplicates, and request deletion through the selected storage provider. A
provider may refuse deletion; Android deletion is not moved to a trash folder and can be permanent.

To build the Android APK, install the Android SDK (API 26 or newer) and NDK (r30 or newer),
install `cargo-apk2`, and add the `aarch64-linux-android` Rust target:

```sh
cargo install cargo-apk2
rustup target add aarch64-linux-android
cargo apk2 build --release
```

GitHub Actions builds a Windows x86_64 executable and an Android arm64 APK on pushes, pull
requests, and manual runs. Download them from the run's **Artifacts** section; artifacts are kept
for 14 days. The APK is an unsigned development build, and Android provider behavior still needs
testing on a device.
