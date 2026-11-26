# Changelog

## [0.6.1](https://github.com/dtorrensf/pmp/compare/pmp-v0.6.0...pmp-v0.6.1) (2026-11-22)


### Bug Fixes

* Move description cursos ([#42](https://github.com/dtorrensf/pmp/issues/42)) ([a17a8ae](https://github.com/dtorrensf/pmp/commit/a17a8aedffed9573c26c3e3cedf796d7870e8082))

## [0.6.0](https://github.com/dtorrensf/pmp/compare/pmp-v0.5.0...pmp-v0.6.0) (2026-11-17)


### Features

* Adds version management and improves UI components ([#40](https://github.com/dtorrensf/pmp/issues/40)) ([8864ef1](https://github.com/dtorrensf/pmp/commit/8864ef11cf2e0ce464db2d79499a7f182c6e5f40))

## [Unreleased]

### Features

* Add version compaction: freeze a released version into a summary snapshot, review it from the TUI and export it as Markdown with Mermaid charts.
* Add a semantic visual theme: a single source of truth for colors with `NO_COLOR` and `PMP_THEME` support.
* Add shared visual primitives (glyph set, badges, progress bars and empty states) with an ASCII fallback via `PMP_ASCII`.
* Add subtle motion: eased progress gauges and bars, a braille spinner next to in-progress tasks, toasts that slide in, a short celebration when a version reaches 100% or is compacted, and a `PMP_REDUCED_MOTION` switch to turn it all off.
* Add a top bar with the app version, a centered breadcrumb of the current context and the update badge. The terminal window title follows the breadcrumb and is restored on exit.
* Add `pmp config` to inspect every effective configuration value with the layer that produced it and to edit `config.toml` preserving comments and format, without starting the TUI or the MCP server. Agent profile `env` secrets are hidden and cannot be edited through the command.
* Prefill the new project's working directory with the directory `pmp` was started from, so creating a project no longer needs typing the checkout path by hand. Editing a project always shows its saved directory.

## [0.5.0](https://github.com/dtorrensf/pmp/compare/pmp-v0.4.1...pmp-v0.5.0) (2026-10-25)


### Features

* Implement version management features and scrolling enhancements ([#34](https://github.com/dtorrensf/pmp/issues/34)) ([4180510](https://github.com/dtorrensf/pmp/commit/4180510e87366889a0f54851d680b80eb1846533))

## [0.4.1](https://github.com/dtorrensf/pmp/compare/pmp-v0.4.0...pmp-v0.4.1) (2026-10-17)


### Bug Fixes

* Test implementation ([#27](https://github.com/dtorrensf/pmp/issues/27)) ([491cf63](https://github.com/dtorrensf/pmp/commit/491cf63d2a1361f17d6feb4b57578ce58ec0beab))

## [0.4.0](https://github.com/dtorrensf/pmp/compare/pmp-v0.3.0...pmp-v0.4.0) (2026-10-17)


### Features

* Add comment on README ([#25](https://github.com/dtorrensf/pmp/issues/25)) ([ce289e3](https://github.com/dtorrensf/pmp/commit/ce289e3d47596e257654523ed5f7115b55a9354b))

## [0.3.0](https://github.com/dtorrensf/pmp/compare/pmp-v0.2.0...pmp-v0.3.0) (2026-10-12)


### Features

* Add version and help options to CLI ([#19](https://github.com/dtorrensf/pmp/issues/19)) ([71bcebc](https://github.com/dtorrensf/pmp/commit/71bcebc1ff6fa764109ae8eb45911adcda531198))
* Version and help commands ([#20](https://github.com/dtorrensf/pmp/issues/20)) ([beb7fe4](https://github.com/dtorrensf/pmp/commit/beb7fe48082faafef3779506ced3cc0b80e391ff))

## [0.2.0](https://github.com/dtorrensf/pmp/compare/pmp-v0.1.0...pmp-v0.2.0) (2026-10-12)


### Features

* Add release docs and config ([889168e](https://github.com/dtorrensf/pmp/commit/889168e660ee2ef6bd0b66c5ca762a5a83e62c1c))
* Add repository real address ([b1c5ee9](https://github.com/dtorrensf/pmp/commit/b1c5ee9b340d3ed853e2debd5db95d15b07658a8))
* Enable CI workflow ([cc2743f](https://github.com/dtorrensf/pmp/commit/cc2743f0998bd02d6c860ae5dccaa64d7dde0d3d))
* First version commit ([cc42de7](https://github.com/dtorrensf/pmp/commit/cc42de7f4f128d03c7b7f63c3ad08e7657cc8acf))


### Bug Fixes

* Chore bump version automation flow ([#11](https://github.com/dtorrensf/pmp/issues/11)) ([b3276fe](https://github.com/dtorrensf/pmp/commit/b3276fe80c438ae3d5943511ccae50d9e0889dc3))
* GH automerge workflow ([#13](https://github.com/dtorrensf/pmp/issues/13)) ([2a0b4f0](https://github.com/dtorrensf/pmp/commit/2a0b4f01232e38d073479eb9c4373e741cc36db9))
* Remove automerge ([#15](https://github.com/dtorrensf/pmp/issues/15)) ([b315b72](https://github.com/dtorrensf/pmp/commit/b315b725cfbaf4a7fcace833b8c792e65c902488))
