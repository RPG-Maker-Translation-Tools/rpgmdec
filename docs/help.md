# Help Documentation

RPGMDec is extremely easy to use. You can start by dropping an encrypted RPG Maker archive or assets into the application window.

Imported entries will appear in a list at the left side of the screen.

## Decryption

Once you've imported an encrypted archive or encrypted assets, program will enter decryption state. In this state, it will display `Decrypt` button in the bottom-left side of the window.

Decryption requires set output directory. You can input it manually or locate by pressing the `Select Output Directory` button.

If you need only some specific assets, you can select only those for decryption. If you want to confirm whether you selected the right ones, you can select the file and inspect it, images, audio and fonts are supported.

From here you just need to selected files you want to decrypt and press the magic button.

## Encryption

For encryption, you unfortunately can't just drop all assets and expect it to work. Assets are require the right structure for encryption to work:

-   If you want to encrypt an archive, path to the file should have Audio/Graphics/Fonts/Data directory.
-   If you want to encrypt assets, path to the file should have www directory.

Once you've imported assets, program will enter encryption state. In this state, it will display `Encrypt` button in the bottom-left side of the window.

Encryption requires set output directory and output engine. You can input output directory manually or locate by pressing the `Select Output Directory` button, and you can select output engine by selecting it in a drop-down selector.

Also, you'll need a key for asset encryption. You can input it manually if you know it or load from other encrypted asset.

For archive encryption, available engines are XP, VX and VX Ace.

For asset encryption, available engines are MV and MZ. Only images and audio are encrypted, as RPG Maker does it: fonts and other files of the game stay as they are and are not listed.

### File name encoding

File names inside archives are read as UTF-8 by default. Entries whose names are not valid UTF-8 are skipped, and a message tells you how many. Archives of older games made on Japanese systems usually need Shift-JIS: pick it in the `Encoding` menu, and the opened archive is read again.

The same menu sets the encoding the names are written in when you encrypt an archive. A name that can't be written in it stops the encryption instead of being stored damaged.

If you need only some specific assets, you can select only those for encryption. If you want to confirm whether you selected the right ones, you can select the file and inspect it, images, audio and fonts are supported.

From here you just need to selected files you want to decrypt and press the magic button.
