# rpgmdec

<p align="center">
    <img src="./icons/256px.png" alt="Description" width="256"/>
</p>

RPGMDec - это универсальный, лёгкий (размером в 1 МБ) и быстрый графический интерфейс для дешифровки архивов RPG Maker XP/VX/VXAce (`.rgssad`/`.rgss2a`/`.rgss3a`) и ассетов RPG Maker MV/MZ (`.rpgmvp`/`.png_`, `.rpgmvo`/`ogg_`, `.rpgmvm`/`m4a_`).

Он также поддерживает зашифровку вышеупомянутых архивов и ассетов.

`rpgmdec` основан на **НЕВЕРОЯТНО БЫСТРЫХ** 🔥🔥 библиотеках 🦀🦀🦀 Rust:

- [rpgm-asset-decrypter-lib](https://github.com/rpg-maker-translation-tools/rpgm-asset-decrypter-lib)
- [rpgm-archive-decrypter-lib](https://github.com/rpg-maker-translation-tools/rpgm-archive-decrypter-lib)

![Превью аудио](./screenshots/audio.png)
![Превью изображения](./screenshots/image.png)

## Установка

Скачивайте бинарные файлы в секции **Releases**. Для их работы нужна поддержка **[x86-64-v2](https://en.wikipedia.org/wiki/X86-64#Microarchitecture_levels)** и **AES-NI**.

## Использование

Ознакомьтесь с [Помощью](./docs/help-ru.md).

## Поддерживаемые форматы

Помимо того, что программа позволяет зашифровывать/дешифровывать ассеты, она ещё позволяет проигрывать аудио, осматривать изображения **И** даже осматривать шрифты `ttf`/`otf`.

### Дешифровка

- Архивы: `.rgssad`, `.rgss2a`, `.rgss3a`
  - Данные: `rxdata`, `rvdata`, `rvdata2`
  - Изображения: `png`
  - Аудио: `ogg`, `m4a`
  - Шрифты: `ttf`, `otf`
- Ассеты: `.rpgmvp`/`.png_`, `.rpgmvo`/`ogg_`, `.rpgmvm`/`m4a_`

### Зашифровка

- Данные: `rxdata`, `rvdata`, `rvdata2`
- Изображения: `png`
- Аудио: `ogg`, `m4a`
- Шрифты: `ttf`, `otf`

## Поддержка

[Я](https://github.com/savannstm), ответственный за этот проект - бедный студент колледжа из Восточной Европы.

Если вы можете, пожалуйста, рассмотрите возможность поддержать нас через:

- [Ko-fi](https://ko-fi.com/savannstm)
- [Patreon](https://www.patreon.com/cw/savannstm)
- [Boosty](https://boosty.to/mcdeimos)

Даже если вы не поддержите, всё нормально. Мы продолжим заниматься тем же, что делаем сейчас.

## Лицензия

Проект лицензирован под WTFPL.
