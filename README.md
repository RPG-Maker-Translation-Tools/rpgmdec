# rpgmdec

[README на русском](./README-ru.md)

<div style="display: flex; align-items: center; justify-content: center">
    <img src="./icons/128px.png" alt="rpgmdec logo" width="128"/>
</div>

RPGMDec is a universal, lightweight (1 MB executable) GUI for decrypting and encrypting RPG Maker archives and assets.

File names inside archives are read as UTF-8; use the `Encoding` menu (e.g. Shift-JIS) for archives of older Japanese games.

`rpgmdec` is built on these Rust libraries:

- [rpgm-asset-decrypter-lib](https://github.com/rpg-maker-translation-tools/rpgm-asset-decrypter-lib)
- [rpgm-archive-decrypter-lib](https://github.com/rpg-maker-translation-tools/rpgm-archive-decrypter-lib)

![Audio preview](./screenshots/audio.png)
![Image preview](./screenshots/image.png)

## Installation

Get the binaries from **Releases**. They require **[x86-64-v2](https://en.wikipedia.org/wiki/X86-64#Microarchitecture_levels)** and **AES-NI** support.

## Usage

See [Help](./docs/help.md).

## Supported Formats

Besides decrypting and encrypting, the program can play audio and preview images and `ttf`/`otf` fonts.

### Decrypt

- Archives: `.rgssad`, `.rgss2a`, `.rgss3a`
  - Data: `rxdata`, `rvdata`, `rvdata2`
  - Images: `png`
  - Audio: `ogg`, `m4a`
  - Fonts: `ttf`, `otf`
- Assets: `.rpgmvp`/`.png_`, `.rpgmvo`/`ogg_`, `.rpgmvm`/`m4a_`

### Encrypt

- Data: `rxdata`, `rvdata`, `rvdata2`
- Images: `png`
- Audio: `ogg`, `m4a`
- Fonts: `ttf`, `otf`

## Building

You need Rust `1.88`, CMake and a C++ compiler, since FLTK is built from source by default (see `system-fltk` below). On Linux, the packages to install are listed in `.github/workflows/release.yml`.

```bash
git clone https://github.com/RPG-Maker-Translation-Tools/rpgmdec
cd rpgmdec
cargo build --release
```

The binary is `target/release/rpgmdec` (`rpgmdec.exe` on Windows).

### Features

| Feature           | Default | Effect                                                                                                  |
| ----------------- | ------- | ------------------------------------------------------------------------------------------------------- |
| `gxhash`          | yes     | Uses gxhash for the app's hash maps. Needs AES-NI (or Armv8 crypto).                                    |
| `rapidhash`       | no      | Uses rapidhash instead. Only applies when `gxhash` is off.                                              |
| `mimalloc`        | yes     | Uses the mimalloc allocator.                                                                            |
| `system-fltk`     | no      | Links an installed FLTK 1.4+ found by CMake, instead of building the bundled copy.                      |
| `system-png`      | no      | Uses the system libpng instead of FLTK's bundled one.                                                   |
| `system-jpeg`     | no      | Uses the system libjpeg instead of FLTK's bundled one.                                                  |
| `system-zlib`     | no      | Uses the system zlib instead of FLTK's bundled one.                                                     |
| `static-msvcrt`   | no      | Links the MSVC C runtime statically. Used by the Windows release build.                                 |
| `use-ninja`       | no      | Builds FLTK with the Ninja generator when `ninja` is on `PATH`. MSVC builds pick Ninja the same way.   |

Turn off the defaults to build with rapidhash instead of gxhash, and add `mimalloc` back if you want it:

```bash
cargo build --release --no-default-features --features rapidhash,mimalloc
```

`--features` adds to the defaults, e.g. `cargo build --release --features system-zlib`.

## Support

[Me](https://github.com/savannstm), the maintainer of this project, is a poor college student from Eastern Europe.

If you could, please consider supporting us through:

- [Boosty](https://boosty.to/mcdeimos) (has lower fees)
- [Ko-fi](https://ko-fi.com/savannstm)

Even if you don't, we will still continue working on the project anyways.

## License

Licensed under [WTFPL](https://www.wtfpl.net/).
