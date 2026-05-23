<!-- Instructions

This changelog follows the patterns described here: <https://keepachangelog.com/en/>.

Subheadings to categorize changes are `added, changed, deprecated, removed, fixed, security`.

-->

# Changelog

The latest published `imaging_vello` release is [0.0.1](#001-2026-05-21), which was released on
2026-05-21. You can find its changes [documented below](#001-2026-05-21).

## [Unreleased]

### Changed

- Updated the backend to Vello 0.9 and `wgpu` 29. ([#64][] by [@waywardmonkeys][])
- Kept the `vello-0-7` and `vello-0-8` compatibility features for integrations that still use
  `wgpu` 27 or 28. ([#64][] by [@waywardmonkeys][])
- Enabled glyph brush transforms through the Vello 0.9 glyph-run API.
  ([#64][] by [@waywardmonkeys][])

## [0.0.1][] (2026-05-21)

This release has an [MSRV][] of 1.92.

This is the initial release.

[@waywardmonkeys]: https://github.com/waywardmonkeys

[#64]: https://github.com/forest-rs/imaging/pull/64

[Unreleased]: https://github.com/forest-rs/imaging/compare/imaging_vello-v0.0.1...HEAD
[0.0.1]: https://github.com/forest-rs/imaging/releases/tag/imaging_vello-v0.0.1

[MSRV]: README.md#minimum-supported-rust-version-msrv
