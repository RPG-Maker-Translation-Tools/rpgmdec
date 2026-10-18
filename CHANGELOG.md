## 1.2.0

- Renamed the `Add File` and `Add Folder` menu items to `Open File` and `Open Folder`.
- Fixed the folder picker from `Open Folder` being titled `Select File`.
- Dropping unsupported files no longer clears the list you already have.
- Encrypting an archive uses less memory, since files are read while the archive is written.
- A failed archive encryption no longer leaves a half-written archive behind.
- Decryption and encryption now stop with an error when two files would go to the same output file.
- Fixed archive encryption using wrong paths when a folder above the game is named `Data` or `Graphics`.
- Starting playback of a selected audio file no longer reads the file a second time.
- Fixed a track that ended stopping the next track started right after it.
- Fixed seeking with the audio progress slider.
- Zooming an image now starts from its real size, so the first zoom step no longer jumps.
- Opening another image resets its zoom and position.
- Fixed some glyphs being drawn at the wrong height in font previews.
- Fixed a crash when the home folder can't be found.
- Fixed mouse wheel zoom not scaling the image.
- Decrypting an archive now stops with an error when two entries share a name, and rejects names containing `:`.
- The wait cursor shows while files are loading, decrypting or encrypting.
- Fixed the Russian text of the file dialog filter.
- Switching language no longer replaces a file's message with the welcome text.
- Pressing Play during playback no longer restarts the track.
- A failed archive reload no longer leaves the previous archive's data in memory.

### From rpgmad-lib v7.0.0

- **Breaking:** `Decrypter` is removed. Use `decrypt` to read an archive and `encrypt` or `encrypt_to_vec` to write one.
- **Breaking:** `Engine::VXAce` is now `Engine::Vxace`, and `Engine.VXAce` is now `Engine.Vxace` in JavaScript. Rename it in your code.
- **Breaking:** The C `rpgmard_encrypt` and `rpgmard_encrypted_buffer_size` functions no longer take a scratch buffer, and the decrypter size and alignment functions are removed.
- **Breaking:** `ExtractError` and `EncryptError` are now `non_exhaustive`, and `Truncated` and `Corrupt` carry a byte offset. Update matches on them.
- Added streaming decryption and encryption for Rust, so archives can be read and written without loading them fully into memory.
- Added `encrypt_to_vec`, which returns a finished archive without sizing a buffer by hand.
- Added a way to get an engine from a file extension such as `.rgss3a`.
- Added checks for archive paths that would escape the output folder, such as `..` components or absolute paths.
- Added an `alloc` feature so `no_std` projects can use `encrypt_to_vec`.
- Added `rpgmard_status_message` to C, which describes any status code.
- Added `rpgmard_decrypter_entries_left` to C, which reports how many entries remain.
- Fixed `encryptArchive` in JavaScript returning a zero-filled buffer for archives over 4 GiB. It now throws.
- Sped up decrypting and encrypting archives.
- Archives with more than 1,048,576 entries are now rejected with an error.

### From rpgm-asset-decrypter-lib v5.0.0

- **Breaking:** `FileType` variants are renamed to `Png`, `Ogg` and `M4a`. Update the names in your code.
- **Breaking:** `encrypt` and `encrypt_in_place` now take a `FileType` as their second argument, and the data is checked against it.
- **Breaking:** `FileType` and `Error` are now `#[non_exhaustive]`. Add a wildcard arm to `match` statements on them.
- **Breaking:** `DECRYPTED_ASSETS_EXTS` is renamed to `DECRYPTED_ASSET_EXTS`.
- **Breaking:** Converting a string to a `FileType` now fails with `Error::UnsupportedExtension` instead of a string error.
- Decrypting a file encrypted with a different key than the decrypter's now returns `KeyMismatch` instead of corrupt output.
- Encrypting data that doesn't look like its declared file type now fails with `InvalidContent`.
- Parsing a `FileType` from an extension now ignores case, so `RPGMVP` is accepted.
- Added `decrypt_keyed` and `decrypt_in_place_keyed`, which take `&self`, so a keyed decrypter can be shared between threads.
- Added `decrypted_extension` and `encrypted_extension` to `FileType`.
- `key()` now always returns lowercase hex, even if the key was set in uppercase.
- The free `decrypt_in_place` function now returns the decrypted data as a slice.
