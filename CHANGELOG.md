
## [0.1.14](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.13...v0.1.14) - 2026-11-02

### Bug Fixes

- *(watch)* Stop the watcher counting a scan's own reads ([#50](https://github.com/pkgforge-apps/blue-dove/pull/50)) - ([a69344e](https://github.com/pkgforge-apps/blue-dove/commit/a69344e14d2078c02b0c81641433e4091c25b1d0))

## [0.1.13](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.12...v0.1.13) - 2026-11-02

### Bug Fixes

- Fonts, cover resolution, MPRIS in preview, and the library scan (#43, #44, #45, #46) ([#47](https://github.com/pkgforge-apps/blue-dove/pull/47)) - ([e3bb9d2](https://github.com/pkgforge-apps/blue-dove/commit/e3bb9d20c523785cb9068cbdc2a4480a286a9aaa))

## [0.1.12](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.11...v0.1.12) - 2026-11-01

### Performance

- Decode album art on demand instead of during the scan ([#42](https://github.com/pkgforge-apps/blue-dove/pull/42)) - ([cf9aecd](https://github.com/pkgforge-apps/blue-dove/commit/cf9aecd671bbb781ef6e67d070e6b1af04ba33d6))

## [0.1.11](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.10...v0.1.11) - 2026-10-31

### Bug Fixes

- Pitch after a surround track, and fonts for Arabic titles ([#38](https://github.com/pkgforge-apps/blue-dove/pull/38)) - ([bc44bc2](https://github.com/pkgforge-apps/blue-dove/commit/bc44bc26db8c69bec91d566477b9c79be29a2884))

## [0.1.10](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.9...v0.1.10) - 2026-10-31

### Miscellaneous Tasks

- *(appimage)* Build the image in the shared release job ([#30](https://github.com/pkgforge-apps/blue-dove/pull/30)) - ([3e24856](https://github.com/pkgforge-apps/blue-dove/commit/3e24856b9610816fc8c03519933f64fb05cd4680))

## [0.1.9](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.8...v0.1.9) - 2026-10-31

### Features

- Add a blind comparison of two tracks ([#27](https://github.com/pkgforge-apps/blue-dove/pull/27)) - ([9cb5a24](https://github.com/pkgforge-apps/blue-dove/commit/9cb5a24af62159f5706409c83defd37d2364f998))

### Bug Fixes

- *(window)* Send no max size for the full window ([#25](https://github.com/pkgforge-apps/blue-dove/pull/25)) - ([ce07dc7](https://github.com/pkgforge-apps/blue-dove/commit/ce07dc7d5a8d942ed999bfaaa9d14131b1418bb0))
- Music directory files, Wayland close, MPRIS art and surround downmix ([#29](https://github.com/pkgforge-apps/blue-dove/pull/29)) - ([0555065](https://github.com/pkgforge-apps/blue-dove/commit/0555065df0f6439e8d49888263d5ffb47b46d9bd))

## [0.1.8](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.7...v0.1.8) - 2026-10-30

### Features

- *(packaging)* Offer the player for every format it reads ([#20](https://github.com/pkgforge-apps/blue-dove/pull/20)) - ([2363ab9](https://github.com/pkgforge-apps/blue-dove/commit/2363ab934d15d9aadaa6afb8062a1508f340cce8))

## [0.1.7](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.6...v0.1.7) - 2026-10-30

### Bug Fixes

- *(theme)* Read the desktop from XDG_CURRENT_DESKTOP ([#14](https://github.com/pkgforge-apps/blue-dove/pull/14)) - ([0dfb13f](https://github.com/pkgforge-apps/blue-dove/commit/0dfb13f86b01bbe4e0d900e73cf8b819be791658))

## [0.1.6](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.5...v0.1.6) - 2026-10-30

### Features

- *(theme)* Add Nord, Catppuccin and Solarized - ([2bf1d74](https://github.com/pkgforge-apps/blue-dove/commit/2bf1d74a678402f1011a640394ac4893cbef9f07))

## [0.1.5](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.4...v0.1.5) - 2026-10-30

### Features

- *(decode)* Play AMR call recordings - ([b96ff8c](https://github.com/pkgforge-apps/blue-dove/commit/b96ff8c7ad792d5c14fb5b706ba31ad94cb84339))
- *(ui)* Preview a single file from the command line or a drop - ([8bdce35](https://github.com/pkgforge-apps/blue-dove/commit/8bdce3542dff21db7b36db8bc8b2852a20d34285))

### Bug Fixes

- *(ui)* Floor the popup at the size it picked - ([6ace491](https://github.com/pkgforge-apps/blue-dove/commit/6ace491dd88d744421aef40eeb2808eb1f61f956))
- *(ui)* Size the popup to its art and let it resize - ([74b5d56](https://github.com/pkgforge-apps/blue-dove/commit/74b5d568fbb031ca230103ce0b800728d5847792))
- *(ui)* Pin the preview window to one size - ([d19241f](https://github.com/pkgforge-apps/blue-dove/commit/d19241fd5b9c6bef4a158370edd9ec7131b4ba05))
- *(ui)* Preview a file opened while the player runs - ([335ceaf](https://github.com/pkgforge-apps/blue-dove/commit/335ceafca35e99864ad950d9ff5d095a06950baf))
- *(ui)* Resize the preview and drop the empty art box - ([6137d97](https://github.com/pkgforge-apps/blue-dove/commit/6137d97fa1a70d8a992b4f1842aa7a249747d6f2))

## [0.1.4](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.3...v0.1.4) - 2026-10-29

### Features

- *(ui)* Add every album in the library from its header - ([843120c](https://github.com/pkgforge-apps/blue-dove/commit/843120c6a89528a0700c5559f0cfde410c4eef3c))

## [0.1.3](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.2...v0.1.3) - 2026-10-29

### Miscellaneous Tasks

- *(appimage)* Retry each pacman transaction three times - ([447e641](https://github.com/pkgforge-apps/blue-dove/commit/447e641d3079839440cb78ca853474e5b2f8377d))

## [0.1.2](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.1...v0.1.2) - 2026-10-29

### Features

- Search the playlist and walk it from the keyboard - ([cf89af8](https://github.com/pkgforge-apps/blue-dove/commit/cf89af8a1cd89f246ee752cd369d75fe9080c685))

### Bug Fixes

- *(fonts)* Split the XDG fallback as a list, not a path - ([c9a5e89](https://github.com/pkgforge-apps/blue-dove/commit/c9a5e893b9be43ee92e54cc21083cbc2103b002e))

### Documentation

- Say what the AppImage runs on, and when it is built - ([f6fa628](https://github.com/pkgforge-apps/blue-dove/commit/f6fa6289738082a2b3cf1595413d81ed377765a3))

### Miscellaneous Tasks

- Sync shared guidelines ([#5](https://github.com/pkgforge-apps/blue-dove/pull/5)) - ([70ea0b8](https://github.com/pkgforge-apps/blue-dove/commit/70ea0b85dd7556ab6f7c256ba169cee9d8ed63f4))

## [0.1.1](https://github.com/pkgforge-apps/blue-dove/compare/v0.1.0...v0.1.1) - 2026-10-29

### Bug Fixes

- *(fonts)* Find system fonts through the XDG directories - ([cf789a7](https://github.com/pkgforge-apps/blue-dove/commit/cf789a73522f299440cd59e7aa3e18493d653252))
