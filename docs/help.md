# Help Documentation

RPGMDec is easy to use: drop an encrypted RPG Maker archive, its assets, or a folder containing them into the window.

Imported entries appear in a list on the left.

## Decryption

After importing encrypted archives or assets, the program enters decryption state and shows a `Decrypt` button in the bottom-left corner.

Decryption requires an output directory: type it in or pick it with the `Select Output Directory` button.

To decrypt only some assets, select them. To check a selection, select a file to inspect it; images, audio and fonts are supported.

Then press the magic button.

## Encryption

Encryption needs assets in the right directory structure:

-   To encrypt an archive, its path must include an Audio/Graphics/Fonts/Data directory.
-   To encrypt assets, its path must include a www directory.

After importing assets, the program enters encryption state and shows an `Encrypt` button in the bottom-left corner.

Encryption requires an output directory and an output engine. Type the directory in or pick it with the `Select Output Directory` button, and choose the engine from the drop-down.

Asset encryption also needs a key: type it in if you know it, or load it from another encrypted asset.

For archive encryption, available engines are XP, VX and VX Ace.

For asset encryption, available engines are MV and MZ. As in RPG Maker, only images and audio are encrypted; fonts and other game files stay as they are and aren't listed.

### File name encoding

File names are read as UTF-8 by default. Entries whose names aren't valid in the selected encoding are skipped, and a message says how many. Archives of older Japanese games usually need Shift-JIS: pick it in the `Encoding` menu and the archive is read again.

The same menu sets the encoding names are written in when encrypting an archive. A name that can't be encoded stops encryption rather than being stored damaged.
