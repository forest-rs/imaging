<!-- Instructions

This changelog follows the patterns described here: <https://keepachangelog.com/en/>.

Subheadings to categorize changes are `added, changed, deprecated, removed, fixed, security`.

-->

# Changelog

The latest published `imaging` release is [0.0.1](#001-2026-05-21), which was released on
2026-05-21. You can find its changes [documented below](#001-2026-05-21).

## [Unreleased]

### Added

- Isolated groups can retain a filter graph with `GroupRef::with_filter_graph`. Filter graphs
  describe filter regions, primitive inputs, color interpolation, and a selected output.

### Changed

- For struct literals and direct assignment, wrap `GroupRef::filters` in
  `FilterEffectsRef::Chain` and `record::Group::filters` in `FilterEffects::Chain`.
  `GroupRef::with_filters` accepts a simple filter slice.

## [0.0.1][] (2026-05-21)

This release has an [MSRV][] of 1.92.

This is the initial release.

[Unreleased]: https://github.com/forest-rs/imaging/compare/imaging-v0.0.1...HEAD
[0.0.1]: https://github.com/forest-rs/imaging/releases/tag/imaging-v0.0.1

[MSRV]: README.md#minimum-supported-rust-version-msrv
