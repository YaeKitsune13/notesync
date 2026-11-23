# NoteSync

Локальная система синхронизации Markdown-заметок с поддержкой UTF-8 (включая CJK / пиньинь), разрешением конфликтов и автопоиском сервера в локальной сети. Весь стек написан на Rust.

## Архитектура

- **`notesync-server`** — HTTP API сервер (`axum` + `SQLite` в WAL-режиме) с ревизиями (`rev`), журналом событий, версионированием при конфликтах и анонсом сервиса в локальной сети через mDNS (`_notesync._tcp.local.`).
- **`notesync-desktop`** — десктопный клиент для Linux на `iced` (тема Catppuccin Mocha, иконки Lucide). Работает напрямую поверх HTTP API без локальной БД.
- **`notesync-mobile`** — Android-клиент на `Dioxus` с локальным офлайн-кэшем в `SQLite`, автопоиском сервера через `mdns-sd` и фоновой синхронизацией.

## Быстрая установка (GitHub Releases)

Скачайте готовые файлы из раздела **[Releases](https://github.com/YaeKitsune13/notesync/releases)**:

### 1. Сервер (Linux x86_64)

```bash
mkdir -p ~/.local/bin ~/.local/share/notesync ~/.config/systemd/user
mv notesync-server-linux-x86_64 ~/.local/bin/notesync-server
chmod +x ~/.local/bin/notesync-server

cat << 'EOF' > ~/.config/systemd/user/notesync.service
[Unit]
Description=NoteSync Local Synchronization Server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=%h/.local/share/notesync
ExecStart=%h/.local/bin/notesync-server
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
systemctl --user enable --now notesync.service
```

При первом запуске сервер создаст базу `notes.db` и файл `~/.local/share/notesync/notes.db.token` с Bearer-токеном авторизации. По умолчанию слушает `0.0.0.0:8787`.

### 2. Десктоп-клиент (Linux x86_64)

```bash
mv notesync-desktop-linux-x86_64 ~/.local/bin/notesync-desktop
chmod +x ~/.local/bin/notesync-desktop
notesync-desktop

```

При первом запуске укажите адрес сервера (например, `[http://127.0.0.1:8787](http://127.0.0.1:8787)`) и токен из `notes.db.token` на вкладке настроек. Конфигурация сохраняется в `~/.config/notesync-desktop.json`.

### 3. Android-клиент (arm64)

Установите `notesync-mobile-android-arm64.apk` на смартфон (или через `adb install notesync-mobile-android-arm64.apk`). В локальной сети приложение автоматически найдёт сервер по mDNS — достаточно ввести токен в настройках.

## HTTP API

Все запросы (кроме `/health`) требуют заголовок `Authorization: Bearer <token>`.

| Метод    | Эндпоинт                                   | Описание                                                      |
| -------- | ------------------------------------------ | ------------------------------------------------------------- |
| `GET`    | `/health`                                  | Проверка доступности сервера                                  |
| `GET`    | `/notes?since=<seq>`                       | Инкрементальное получение заметок                             |
| `GET`    | `/notes/:id`                               | Получение конкретной заметки                                  |
| `PUT`    | `/notes/:id`                               | Создание или обновление с проверкой `base_rev`                |
| `DELETE` | `/notes/:id?base_rev=<rev>&device_id=<id>` | Мягкое удаление заметки                                       |
| `GET`    | `/notes/:id/versions`                      | История версий и неразрешённых конфликтов                     |
| `POST`   | `/notes/:id/conflicts/:vid/resolve`        | Разрешение конфликта (`keep_server`, `use_this`, `keep_both`) |
| `GET`    | `/log?limit=100`                           | Журнал событий синхронизации                                  |

## Сборка из исходников

```bash
# Сервер
cd notesync-server && cargo build --release

# Десктоп-клиент
cd notesync-desktop && cargo build --release

# Android APK (требуются dioxus-cli, Android SDK/NDK и target aarch64-linux-android)
cd notesync-mobile && dx build --release --platform android --target aarch64-linux-android

```
