<!-- Instructions

This changelog follows the patterns described here: <https://keepachangelog.com/en/>.

Subheadings to categorize changes are `added, changed, deprecated, removed, fixed, security`.

-->

# Changelog

The latest published `imaging_wgpu` release is [0.0.2](#002-2026-05-30), which was released on
2026-05-30. You can find its changes [documented below](#002-2026-05-30).

## [Unreleased]

## [0.0.2][] (2026-05-30)

This release has an [MSRV][] of 1.92.

### Changed

- Bumped the default renderer lane to `wgpu` 29 while keeping `wgpu` 27 and 28 available through
  explicit version features and modules. ([#64][] by [@waywardmonkeys][])

## [0.0.1][] (2026-05-21)

This release has an [MSRV][] of 1.92.

This is the initial release.

[@waywardmonkeys]: https://github.com/waywardmonkeys

[#64]: https://github.com/forest-rs/imaging/pull/64

[Unreleased]: https://github.com/forest-rs/imaging/compare/imaging_wgpu-v0.0.2...HEAD
[0.0.2]: https://github.com/forest-rs/imaging/compare/imaging_wgpu-v0.0.1...imaging_wgpu-v0.0.2
[0.0.1]: https://github.com/forest-rs/imaging/releases/tag/imaging_wgpu-v0.0.1

[MSRV]: README.md#minimum-supported-rust-version-msrv
